#[cfg(not(target_arch = "wasm32"))]
pub use async_std::task::{spawn, spawn_blocking, JoinHandle};

pub fn block_in_place<F, R>(f: F) -> R
where
    F: FnOnce() -> R,
{
    f()
}

#[cfg(not(target_arch = "wasm32"))]
pub use os::*;

#[cfg(not(target_arch = "wasm32"))]
mod os {
    use async_io::Async;
    use async_std::net::{TcpListener as AsyncListener, TcpStream as AsyncStream, ToSocketAddrs};
    use futures::{AsyncRead as _, Future};
    use std::{
        pin::Pin,
        task::{Context, Poll},
    };
    use tokio::prelude::{AsyncRead, AsyncWrite};
    use tokio_util::compat::{Compat, FuturesAsyncWriteCompatExt};

    #[derive(Debug)]
    pub struct TcpListener(AsyncListener);

    impl TcpListener {
        pub async fn bind<A: ToSocketAddrs>(addrs: A) -> io::Result<TcpListener> {
            Ok(TcpListener(AsyncListener::bind(addrs).await?))
        }

        pub fn local_addr(&self) -> io::Result<SocketAddr> {
            self.0.local_addr()
        }

        pub fn poll_accept(
            &mut self,
            cx: &mut Context,
        ) -> Poll<io::Result<(TcpStream, SocketAddr)>> {
            unsafe {
                Pin::new_unchecked(&mut self.0.accept())
                    .poll(cx)
                    .map_ok(|x| (TcpStream(x.0.compat_write()), x.1))
            }
        }
    }

    #[derive(Debug)]
    pub struct TcpStream(Compat<AsyncStream>);

    impl TcpStream {
        pub fn peer_addr(&self) -> io::Result<SocketAddr> {
            self.0.get_ref().peer_addr()
        }
    }

    impl AsyncRead for TcpStream {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut [u8],
        ) -> Poll<Result<usize, io::Error>> {
            Pin::new(self.0.get_mut()).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for TcpStream {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        #[inline]
        fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    #[cfg(feature = "async-timer")]
    pub use async_std::future::timeout;
    #[cfg(feature = "async-timer")]
    pub use time::*;

    use socket2::Socket;
    use std::{
        io,
        net::{SocketAddr, TcpListener as StdListen},
    };

    pub fn from_std(listen: StdListen) -> io::Result<TcpListener> {
        Ok(TcpListener(AsyncListener::from(listen)))
    }

    pub async fn connect_std(std_tcp: Socket, addr: &SocketAddr) -> io::Result<TcpStream> {
        // Begin async connect and ignore the inevitable "in progress" error.
        std_tcp.set_nonblocking(true)?;
        std_tcp.connect(&(*addr).into()).or_else(|err| {
            // Check for EINPROGRESS on Unix and WSAEWOULDBLOCK on Windows.
            #[cfg(unix)]
            let in_progress = err.raw_os_error() == Some(libc::EINPROGRESS);
            #[cfg(windows)]
            let in_progress = err.kind() == io::ErrorKind::WouldBlock;

            // If connect results with an "in progress" error, that's not an error.
            if in_progress {
                Ok(())
            } else {
                Err(err)
            }
        })?;
        let stream = Async::new(std_tcp.into_tcp_stream())?;

        // The stream becomes writable when connected.
        stream.writable().await?;

        // Check if there was an error while connecting.
        match stream.get_ref().take_error()? {
            None => {
                let tcp = stream.into_inner().unwrap();
                Ok(TcpStream(AsyncStream::from(tcp).compat_write()))
            }
            Some(err) => Err(err),
        }
    }

    #[cfg(feature = "async-timer")]
    mod time {
        use async_io::Timer;
        use futures::{Future, Stream};
        use std::{
            pin::Pin,
            task::{Context, Poll},
            time::{Duration, Instant},
        };

        pub struct Delay(Timer);

        impl Delay {
            pub fn new(duration: Duration) -> Self {
                Delay(Timer::after(duration))
            }
        }

        impl Future for Delay {
            type Output = Instant;

            fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                Pin::new(&mut self.0).poll(cx)
            }
        }

        pub fn delay_for(duration: Duration) -> Delay {
            Delay::new(duration)
        }

        pub struct Interval {
            delay: Delay,
            period: Duration,
        }

        impl Interval {
            fn new(period: Duration) -> Self {
                Self {
                    delay: Delay::new(period),
                    period,
                }
            }
        }

        impl Stream for Interval {
            type Item = ();

            fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<()>> {
                match Pin::new(&mut self.delay).poll(cx) {
                    Poll::Ready(_) => {
                        let dur = self.period;
                        self.delay.0.set_after(dur);
                        Poll::Ready(Some(()))
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
        }

        pub fn interval(period: Duration) -> Interval {
            assert!(period > Duration::new(0, 0), "`period` must be non-zero.");

            Interval::new(period)
        }
    }
}