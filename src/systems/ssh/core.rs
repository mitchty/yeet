use bevy::prelude::*;
use russh::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub struct Ssh;

impl Plugin for Ssh {
    fn build(&self, _app: &mut App) {
        // nop for now maybe forever...
    }
}

// ssh session handle that can be cloned across systems
#[derive(Component, Clone)]
pub struct Session {
    pub handle: Arc<client::Handle<Client>>,
    pub host: String,
    pub user: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("host", &self.host)
            .field("user", &self.user)
            .finish()
    }
}

// Russh Client handler for the ECS
pub struct Client;

#[async_trait::async_trait]
impl client::Handler for Client {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // TODO: Implement proper known_hosts checking
        // For now, log and accept whatever, nobody sane should be using this yet anyway
        debug!("server key: {:?}", server_public_key);
        async { Ok(true) }
    }
}

pub async fn connect(
    host: String,
    user: String,
    key_path: Option<std::path::PathBuf>,
) -> Result<Session, Box<dyn std::error::Error + Send + Sync>> {
    debug!("ssh connecting to {}@{}", user, host);

    let config = client::Config::default();
    let sh = Client {};

    let mut session = client::connect(Arc::new(config), (host.as_str(), 22), sh).await?;

    if let Some(key_path) = key_path {
        info!("attempting key auth with: {:?}", key_path);
        let key_pair = keys::load_secret_key(key_path, None)?;
        let key_with_alg = keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);
        let auth_res = session
            .authenticate_publickey(&user, key_with_alg)
            .await;

        match auth_res {
            Ok(_) => debug!("key authentication successful"),
            Err(e) => return Err(format!("key auth failed: {}", e).into()),
        }
    } else {
        // TODO: Need to allow users/callers to control what keys we will use.
        // Note I am never dealing with passwords for this, you want yeet you
        // use keys end of story.
        let home = std::env::var("HOME")?;
        let key_paths = vec![
            std::path::PathBuf::from(format!("{}/.ssh/id_rsa", home)),
            std::path::PathBuf::from(format!("{}/.ssh/id_ed25519", home)),
        ];

        let mut authenticated = false;
        for path in key_paths {
            if !path.exists() {
                continue;
            }
            debug!("trying key {:?}", path);
            match keys::load_secret_key(&path, None) {
                Ok(key_pair) => {
                    let key_with_alg = keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), None);
                    if session
                        .authenticate_publickey(&user, key_with_alg)
                        .await
                        .is_ok()
                    {
                        debug!("authentication successful with {:?}", path);
                        authenticated = true;
                        break;
                    }
                }
                Err(e) => {
                    info!("failed to load key {:?}: {}", path, e);
                    continue;
                }
            }
        }

        if !authenticated {
            return Err("no valid ssh keys found in ~/.ssh/ that can be abused".into());
        }
    }

    Ok(Session {
        handle: Arc::new(session),
        host: host.clone(),
        user: user.clone(),
    })
}

// Set up port forwarding over the ssh session
// Returns the local port that forwards to remote_port on the remote host
//
// For now pin it to ipv4, should figure out a plan for dual stack 4/6, probably
// just do both always and as long as one works abuse that. Haven't brained that
// out much yet and its not critical right now I'm in "make it work" mode not
// "make it right" mode.
pub async fn setup_port_forward(
    session: Session,
    remote_port: u16,
) -> Result<u16, Box<dyn std::error::Error + Send + Sync>> {
    debug!(
        "setting up port forward to {}:{} via {}@{}",
        "localhost", remote_port, session.user, session.host
    );

    // Let the os give us a random port to bind to on localhost
    let listener = TcpListener::bind("localhost:0").await?;
    let local_addr = listener.local_addr()?;
    let local_port = local_addr.port();

    debug!(
        "local port {} forwarding to remote port {}",
        local_port, remote_port
    );

    // Spawn a background thread to handle sending crap through the forward.
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((local_stream, peer_addr)) => {
                    debug!("accepted connection from {}", peer_addr);
                    let session_handle = session.handle.clone();

                    // Each connection gets a task in tokio too, "just in
                    // case/future mitch will likely think this is right-er than
                    // not"
                    tokio::spawn(async move {
                        if let Err(e) =
                            forward_connection(local_stream, session_handle, remote_port).await
                        {
                            error!("port forwarding error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("failed to accept connection: {}", e);
                    break;
                }
            }
        }
        debug!("port forwarding loop terminated");
    });

    Ok(local_port)
}

// Forward a single TCP connection through ssh, todo udp? Not sure udp over ssh
// tunnel makes sense. I'll probably handle ssh and non ssh traffic separately
// anyway. Also that is a future "make it right" task.
async fn forward_connection(
    local_stream: TcpStream,
    session: Arc<client::Handle<Client>>,
    remote_port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::sync::mpsc;

    let mut channel = session
        .channel_open_direct_tcpip("localhost", remote_port as u32, "localhost", 0)
        .await?;

    debug!("ssh channel opened for forwarding");

    let (mut local_read, mut local_write) = local_stream.into_split();

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let read_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 8192];
        loop {
            match local_read.read(&mut buf).await {
                Ok(0) => {
                    debug!("local connection closed (EOF)");
                    break;
                }
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        error!("failed to queue data for ssh");
                        break;
                    }
                }
                Err(e) => {
                    error!("failed to read from local stream: {}", e);
                    break;
                }
            }
        }
        debug!("local read completed");
    });

    loop {
        tokio::select! {
            // First send any queued data
            Some(data) = rx.recv() => {
                if let Err(e) = channel.data(&data[..]).await {
                    error!("failed to send data over ssh channel: {}", e);
                    break;
                }
            }
            // ditto receive
            Some(msg) = channel.wait() => {
                match msg {
                    ChannelMsg::Data { ref data } => {
                        if let Err(e) = local_write.write_all(data).await {
                            error!("failed to write to local stream: {}", e);
                            break;
                        }
                    }
                    ChannelMsg::Eof => {
                        debug!("remote connection closed (EOF)");
                        break;
                    }
                    ChannelMsg::Close => {
                        debug!("ssh channel closed");
                        break;
                    }
                    _ => {}
                }
            }
            else => break,
        }
    }

    let _ = channel.eof().await;
    let _ = read_handle.await;

    debug!("port forwarding connection completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_ssh_session_is_cloneable() {
        //TODO: write tests for this future mitch, past mitch wants to build a yeet monitor/status client via lightyear first
        // Verify SshSession can be cloned (required for Component)
        // This is a compile-time test - if it compiles, it works... I think, haven't brained long on this yet, this is future brain dump.
    }
}
