use crate::configuration::UdtConfiguration;
use crate::socket::{SocketType, UdtStatus};
use crate::udt::{SocketRef, Udt};
use std::net::SocketAddr;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, Error, ErrorKind, ReadBuf, Result};
use tokio::net::{lookup_host, ToSocketAddrs};

pub struct UdtConnection {
    socket: SocketRef,
}

impl UdtConnection {
    pub(crate) fn new(socket: SocketRef) -> Self {
        Self { socket }
    }

    pub async fn connect(
        addr: impl ToSocketAddrs,
        config: Option<UdtConfiguration>,
    ) -> Result<Self> {
        Self::_bind_and_connect(None, addr, config).await
    }

    pub async fn bind_and_connect(
        bind_addr: SocketAddr,
        connect_addr: impl ToSocketAddrs,
        config: Option<UdtConfiguration>,
    ) -> Result<Self> {
        Self::_bind_and_connect(Some(bind_addr), connect_addr, config).await
    }

    async fn _bind_and_connect(
        bind_addr: Option<SocketAddr>,
        addrs: impl ToSocketAddrs,
        config: Option<UdtConfiguration>,
    ) -> Result<Self> {
        let socket = {
            let mut udt = Udt::get().write().await;
            udt.new_socket(SocketType::Stream, config)?.clone()
        };

        let mut last_err = None;
        let mut connected = false;

        for addr in lookup_host(addrs).await? {
            match socket.connect(addr, bind_addr).await {
                Ok(()) => {
                    connected = true;
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        if !connected {
            return Err(last_err.unwrap_or_else(|| {
                Error::new(ErrorKind::InvalidInput, "could not resolve address")
            }));
        }

        loop {
            let status = socket.wait_for_connection().await;
            if status != UdtStatus::Connecting {
                break;
            }
        }
        Ok(Self::new(socket))
    }

    pub async fn send(&self, msg: &[u8]) -> Result<()> {
        self.socket.send(msg)
    }

    pub async fn recv(&self, buf: &mut [u8]) -> Result<usize> {
        let nbytes = self.socket.recv(buf).await?;
        Ok(nbytes)
    }

    pub fn rate_control(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, crate::rate_control::RateControl> {
        self.socket.rate_control.write().unwrap()
    }

    pub async fn close(&self) {
        self.socket.close().await;
    }

    #[must_use]
    pub fn socket_id(&self) -> u32 {
        self.socket.socket_id
    }
}

impl AsyncRead for UdtConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<Result<()>> {
        match self.socket.poll_recv(buf) {
            Poll::Ready(res) => Poll::Ready(res.map(|_| ())),
            Poll::Pending => {
                let waker = cx.waker().clone();
                let socket = self.socket.clone();
                tokio::spawn(async move {
                    socket.wait_for_data_to_read().await;
                    waker.wake();
                });
                Poll::Pending
            }
        }
    }
}

impl AsyncWrite for UdtConnection {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<Result<usize>> {
        let buf_len = buf.len();
        match self.socket.send(buf) {
            Ok(_) => Poll::Ready(Ok(buf_len)),
            Err(err) => match err.kind() {
                ErrorKind::OutOfMemory => {
                    let waker = cx.waker().clone();
                    let socket = self.socket.clone();
                    tokio::spawn(async move {
                        socket.wait_for_next_ack_or_empty_snd_buffer().await;
                        waker.wake();
                    });
                    Poll::Pending
                }
                _ => Poll::Ready(Err(err)),
            },
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.socket.snd_buffer_is_empty() {
            Poll::Ready(Ok(()))
        } else {
            let waker = cx.waker().clone();
            let socket = self.socket.clone();
            tokio::spawn(async move {
                socket.wait_for_next_ack_or_empty_snd_buffer().await;
                waker.wake();
            });
            Poll::Pending
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        if self.socket.status() == UdtStatus::Closed {
            return Poll::Ready(Ok(()));
        }
        let socket = self.socket.clone();
        let waker = cx.waker().clone();
        tokio::spawn(async move {
            socket.close().await;
            waker.wake();
        });
        Poll::Pending
    }
}
