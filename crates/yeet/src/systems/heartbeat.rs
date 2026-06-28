use bevy::prelude::*;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Heartbeat;

impl Plugin for Heartbeat {
    fn build(&self, app: &mut App) {
        app.add_message::<crate::RpcEvent>()
            .add_systems(Startup, startup)
            .add_systems(
                Update,
                (
                    handle_heartbeat_requests,
                    update_heartbeat_timestamps.run_if(bevy::time::common_conditions::on_timer(
                        std::time::Duration::from_secs(1),
                    )),
                ),
            );
    }
}

/// Marker component for SSH connection pair entities
#[derive(Debug, Default, Component)]
pub struct SshConnectionPair;

/// Local hostname for this side of the connection
#[derive(Debug, Component, Deref, Clone)]
pub struct LocalHost(pub String);

/// Remote connection string (e.g., "user@hostname" or "hostname")
#[derive(Debug, Component, Deref, Clone)]
pub struct RemoteConnection(pub String);

/// Last heartbeat timestamp (seconds since UNIX epoch)
#[derive(Debug, Component, Deref, DerefMut, Clone)]
pub struct LastHeartbeat(pub u64);

impl Default for LastHeartbeat {
    fn default() -> Self {
        Self(current_timestamp())
    }
}

/// Get current timestamp in seconds since UNIX epoch
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_secs()
}

/// Initialize the heartbeat system
fn startup(_commands: Commands) {
    info!("heartbeat system starting up");

    // TODO: Get actual hostname - for now use a placeholder
    // In production, we'd use hostname::get() or similar
    let local_hostname = "localhost".to_string();

    info!("local hostname: {}", local_hostname);

    // We'll create connection pair entities as needed via RPC
    // For now, just log that we're ready
    info!("heartbeat system ready to track SSH connection pairs");
}

/// Handle incoming heartbeat RPC requests
fn handle_heartbeat_requests(
    mut events: MessageReader<crate::RpcEvent>,
    runtime: Res<bevy_tokio_tasks::TokioTasksRuntime>,
) {
    use crate::RpcEvent;

    for event in events.read() {
        if let RpcEvent::Heartbeat {
            target,
            response_tx,
        } = event
        {
            info!("Processing heartbeat request for target: {}", target);

            // Clone what we need for the async task
            let target_clone = target.clone();
            let response_tx_clone = response_tx.clone();

            // Spawn async task to handle the heartbeat
            runtime.spawn_background_task(move |_ctx| async move {
                let (success, message) = perform_heartbeat(&target_clone).await;

                // Send response back through the channel
                if let Ok(mut guard) = response_tx_clone.lock()
                    && let Some(tx) = guard.take()
                {
                    let _ = tx.send((success, message));
                }
            });
        }
    }
}

/// Perform the actual heartbeat to the target
async fn perform_heartbeat(target: &str) -> (bool, String) {
    // Parse target - if it contains '@', treat as SSH connection (user@host)
    // Otherwise, it's a local heartbeat
    if target.contains('@') {
        // Remote heartbeat via SSH
        match perform_remote_heartbeat(target).await {
            Ok(msg) => (true, msg),
            Err(e) => (false, format!("Remote heartbeat failed: {}", e)),
        }
    } else {
        // Local heartbeat (just verify local daemon is responsive)
        (true, format!("Local heartbeat to {} OK", target))
    }
}

/// Perform a remote heartbeat via SSH with port forwarding
async fn perform_remote_heartbeat(
    target: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use super::ssh::{connect, setup_port_forward};
    use crate::rpc::yeet::{HeartbeatRequest, yeet_client::YeetClient};

    // Parse user@host format
    let (user, host) = if let Some((u, h)) = target.split_once('@') {
        (u.to_string(), h.to_string())
    } else {
        return Err(format!("Invalid target format '{}'. Expected 'user@host'", target).into());
    };

    // Establish SSH connection (using default SSH key)
    let session = connect(host.clone(), user.clone(), None).await?;

    // Set up port forward to remote yeet daemon (port 50051)
    let local_port = setup_port_forward(session, 50051).await?;

    // Connect to the forwarded port
    let channel = tonic::transport::Channel::from_shared(format!("http://[::1]:{}", local_port))
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?
        .connect()
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;

    let mut client = YeetClient::new(channel);

    // Send a simple heartbeat to the remote daemon
    // Use "localhost" as target since we're already connected to the remote
    let request = tonic::Request::new(HeartbeatRequest {
        target: "localhost".to_string(),
    });

    let response = client
        .heartbeat(request)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })?;
    let reply = response.into_inner();

    if reply.success {
        Ok(format!(
            "Remote daemon at {} responded: {}",
            target, reply.message
        ))
    } else {
        Err(format!("Remote daemon reported failure: {}", reply.message).into())
    }
}

/// Update heartbeat timestamps every second for all connection pairs
fn update_heartbeat_timestamps(
    mut connections: Query<
        (Entity, &LocalHost, &RemoteConnection, &mut LastHeartbeat),
        With<SshConnectionPair>,
    >,
) {
    let now = current_timestamp();

    for (entity, local, remote, mut heartbeat) in connections.iter_mut() {
        let old_timestamp = **heartbeat;
        **heartbeat = now;

        trace!(
            "updated heartbeat for {} -> {} (entity: {:?}): {} -> {}",
            **local, **remote, entity, old_timestamp, now
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_current_timestamp_is_reasonable() {
        let ts = current_timestamp();

        // Should be after 2020 (1577836800) and before 2100 (4102444800)
        assert!(ts > 1577836800, "timestamp should be after 2020");
        assert!(ts < 4102444800, "timestamp should be before 2100");
    }

    #[test]
    fn test_last_heartbeat_default() {
        let hb = LastHeartbeat::default();
        let now = current_timestamp();

        // Should be within 1 second of now
        assert!(
            (*hb as i64 - now as i64).abs() <= 1,
            "default heartbeat should be near current time"
        );
    }

    #[test]
    fn test_components_clone() {
        let local = LocalHost("testhost".to_string());
        let remote = RemoteConnection("user@remotehost".to_string());
        let heartbeat = LastHeartbeat(12345);

        let local_clone = local.clone();
        let remote_clone = remote.clone();
        let heartbeat_clone = heartbeat.clone();

        assert_eq!(*local, *local_clone);
        assert_eq!(*remote, *remote_clone);
        assert_eq!(*heartbeat, *heartbeat_clone);
    }
}
