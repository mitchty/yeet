use crate::systems::netcode::core::*;
use crate::systems::netcode::protocol::ProtocolPlugin;
use bevy::prelude::*;
use core::net::SocketAddr;
use std::time::{Duration, Instant};

use bevy_cronjob::prelude::*;
use lightyear::netcode::Key;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use rand::Rng;

pub struct LightYearClientPlugin;

#[derive(Component, Debug, Clone)]
pub struct ClientAddr(pub String);

#[derive(Component, Debug, Clone)]
pub struct ServerAddr(pub String);

#[derive(Component, Debug, Clone)]
pub struct ClientId(pub u64);

#[derive(Component, Debug, Clone)]
pub struct ReconnectDelay(pub Duration);

#[derive(Component, Debug, Clone)]
pub struct MaxReconnectAttempts(pub u32);

#[derive(Component, Debug)]
pub struct ClientConfig;

#[derive(Bundle)]
pub struct ClientConfigBundle {
    marker: ClientConfig,
    client_addr: ClientAddr,
    server_addr: ServerAddr,
    client_id: ClientId,
    reconnect_delay: ReconnectDelay,
    max_attempts: MaxReconnectAttempts,
}

#[derive(Resource, Debug)]
struct ReconnectionState {
    should_reconnect: bool,
    last_attempt: Option<Instant>,
    attempts: u32,
    disconnected_client: Option<Entity>,
}

impl Plugin for LightYearClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ProtocolPlugin);

        // TODO: need to get this configured from a config file passed into this fn
        app.add_plugins(ClientPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });

        // TODO: I should probably setup a Resource/Component for Connections in
        // general in the ECS. Future sucker mitch problem.
        app.insert_resource(ReconnectionState {
            should_reconnect: false,
            last_attempt: None,
            attempts: 0,
            disconnected_client: None,
        });

        // Observers are fun, need to abuse them more. Also future sucker mitch:
        // TODO: last system should be debug build only.
        app.add_systems(Startup, (spawn_client_config, connect_client).chain());
        app.add_observer(handle_connected);
        app.add_observer(handle_disconnected);
        #[cfg(debug_assertions)]
        app.add_systems(
            Update,
            (
                debug_received_entities.run_if(schedule_passed("every 7 seconds")),
                handle_reconnection,
            ),
        );
    }
}

// Hack way to generate a unique client id based off a v4 uuid by OR'ing the
// top/bottom of the 128bit integer.
fn generate_client_id() -> u64 {
    let uuid = uuid::Uuid::new_v4();
    let bytes = uuid.as_bytes();

    let upper = u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]);
    let lower = u64::from_be_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
    ]);

    let id = upper ^ lower;

    // Ok look, just avoid 0
    if id == 0 { generate_client_id() } else { id }
}

// Need to ensure each local client binds to a uniq port
fn find_available_port() -> std::result::Result<u16, Box<dyn std::error::Error>> {
    use std::net::TcpListener;

    // TODO: a lot of this should be in a config and setup at runtime.
    const PORT_MIN: u16 = 60000;
    const PORT_MAX: u16 = 65535;
    const MAX_ATTEMPTS: u32 = 100;

    let mut rng = rand::thread_rng();

    for _ in 0..MAX_ATTEMPTS {
        let port = rng.gen_range(PORT_MIN..=PORT_MAX);

        match TcpListener::bind(("localhost", port)) {
            Ok(_) => {
                return Ok(port);
            }
            Err(_) => {
                continue;
            }
        }
    }

    Err(format!(
        "failed to find available port after {} attempts to find a port in range {} -> {}",
        MAX_ATTEMPTS, PORT_MIN, PORT_MAX
    )
    .into())
}

fn spawn_client_config(mut commands: Commands) {
    let client_port = match find_available_port() {
        Ok(port) => {
            debug!("using client port: {}", port);
            port
        }
        Err(e) => {
            error!(
                "failed to find available port: {}, falling back to 60000",
                e
            );
            60000
        }
    };

    let client_id = generate_client_id();
    debug!("client id: {}", client_id);

    commands.spawn(ClientConfigBundle {
        marker: ClientConfig,
        client_addr: ClientAddr(format!("localhost:{}", client_port)),
        server_addr: ServerAddr("localhost:5000".to_string()),
        client_id: ClientId(client_id),
        reconnect_delay: ReconnectDelay(Duration::from_secs(2)),
        max_attempts: MaxReconnectAttempts(0),
    });
}

// TODO: UNIT TESTS DUMDUM STOP BEING LAZY
fn parse_addr(addr_str: &str) -> std::result::Result<SocketAddr, Box<dyn std::error::Error>> {
    // Try to parse as a SocketAddr directly first
    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
        return Ok(addr);
    }

    use std::net::ToSocketAddrs;
    let mut addrs = addr_str.to_socket_addrs()?;
    addrs
        .next()
        .ok_or_else(|| format!("Could not resolve address: {}", addr_str).into())
}

// Client connect system. I should abstract all the ssh and lightyear/udp and
// unix domain socket stuff into a plugin I can load for each connection type.
//
// That way I can abuse observers and triggers/messages/events to handle
// connection/reconnection logic and put a pid controller into things there to
// handle control theory.
fn connect_client(
    mut commands: Commands,
    config_query: Query<(&ClientAddr, &ServerAddr, &ClientId), With<ClientConfig>>,
) -> Result {
    let Ok((client_addr, server_addr, client_id)) = config_query.single() else {
        let msg: &str = "client configuration not found! prolly bug";
        error!(msg);
        return Err(anyhow::anyhow!(msg).into());
    };

    info!(
        "client connecting to server at {} with client ID {}",
        server_addr.0, client_id.0
    );

    match attempt_connect(&mut commands, &client_addr.0, &server_addr.0, client_id.0) {
        Ok(_) => {
            debug!("initial connection attempt initiated");
            Ok(())
        }
        Err(e) => {
            error!("failed to initiate initial connection: {:?}", e);
            Err(e)
        }
    }
}

// Mostly for logging, and learning observers/triggers
fn handle_connected(
    trigger: On<Add, Connected>,
    mut reconnect_state: ResMut<ReconnectionState>,
    client_query: Query<Entity, With<Client>>,
) {
    info!(
        "client connected to server successfully {:?}",
        trigger.entity
    );
    debug!("active client entities: {}", client_query.iter().count());

    reconnect_state.should_reconnect = false;
    reconnect_state.attempts = 0;
    reconnect_state.last_attempt = None;
    reconnect_state.disconnected_client = None;
}

// Basically just marks all replicated entities for despawn and to reconnect
//
// That will handle replicating the new client entity associated components.
fn handle_disconnected(
    trigger: On<Add, Disconnected>,
    mut reconnect_state: ResMut<ReconnectionState>,
    mut commands: Commands,
    // TODO: This really needs to be rethought, too much query bs with this approach
    replicated_entities: Query<
        Entity,
        Or<(
            With<crate::systems::netcode::protocol::ReplicatedSource>,
            With<crate::systems::stats::Uptime>,
        )>,
    >,
) {
    warn!("client disconnected from server: {:?}", trigger.entity);

    // Clean up all replicated entities from the previous connection
    // This ensures we start fresh when we reconnect. I think.
    let mut cleaned_count = 0;
    for entity in replicated_entities.iter() {
        // is_ok() is here to avoid panics in the case the entity got despawned
        // behind our backs and then panic() ensues...
        commands.queue(move |world: &mut World| {
            if world.get_entity(entity).is_ok() {
                let _ = world.despawn(entity);
            }
        });
        cleaned_count += 1;
    }

    if cleaned_count > 0 {
        debug!(
            "cleaned up {} replicated entities from previous connection",
            cleaned_count
        );
    }

    // Note: We don't manually despawn the client entity here because lightyear
    // handles that automatically. Manually despawning it causes panics as
    // lightyear may still be applying commands to it.

    reconnect_state.should_reconnect = true;
    reconnect_state.disconnected_client = Some(trigger.entity);
    reconnect_state.last_attempt = None; // Will trigger immediate first retry
    reconnect_state.attempts = 0;

    info!("will retrying connection");
}

fn handle_reconnection(
    mut reconnect_state: ResMut<ReconnectionState>,
    mut commands: Commands,
    active_client_query: Query<Entity, (With<Client>, Without<Disconnected>)>,
    disconnected_client_query: Query<Entity, (With<Client>, With<Disconnected>)>,
    mut config_query: Query<
        (
            Entity,
            &ClientAddr,
            &ServerAddr,
            &mut ClientId,
            &ReconnectDelay,
            &MaxReconnectAttempts,
        ),
        With<ClientConfig>,
    >,
) {
    // TODO: tbh I should probably ecs more and this be a part of a Query....
    if !reconnect_state.should_reconnect {
        return;
    }

    let active_clients = active_client_query.iter().count();
    if active_clients > 0 {
        debug!("skipping reconnection: {} active clients", active_clients);
        return;
    }

    let disconnected_count = disconnected_client_query.iter().count();
    if disconnected_count > 0 {
        debug!("despawning {} disconnected clients", disconnected_count);
        for entity in disconnected_client_query.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    let Ok((
        _config_entity,
        client_addr,
        server_addr,
        mut client_id,
        reconnect_delay,
        max_attempts,
    )) = config_query.single_mut()
    else {
        error!("client configuration not found, bug?");
        return;
    };

    let now = Instant::now();

    let should_attempt = match reconnect_state.last_attempt {
        None => {
            debug!("first reconnection attempt");
            true
        }
        Some(last) => {
            let elapsed = now.duration_since(last);
            if elapsed >= reconnect_delay.0 {
                true
            } else {
                debug!(
                    "waiting for reconnection delay to pass ({:?} < {:?})",
                    elapsed, reconnect_delay.0
                );
                false
            }
        }
    };

    if !should_attempt {
        return;
    }

    if max_attempts.0 > 0 && reconnect_state.attempts >= max_attempts.0 {
        error!(
            "failed to reconnect after {} attempts. I give up for now.",
            reconnect_state.attempts
        );
        reconnect_state.should_reconnect = false;
        return;
    }

    let new_client_id = generate_client_id();
    info!(
        "generating new clientid for reconnection: {} was: {}",
        new_client_id, client_id.0
    );
    client_id.0 = new_client_id;

    reconnect_state.disconnected_client = None;

    reconnect_state.attempts += 1;
    reconnect_state.last_attempt = Some(now);

    info!(
        "attempting reconnection: attempt {}{}",
        reconnect_state.attempts,
        if max_attempts.0 > 0 {
            format!("/{}", max_attempts.0)
        } else {
            String::new()
        }
    );

    match attempt_connect(&mut commands, &client_addr.0, &server_addr.0, new_client_id) {
        Ok(_) => {
            debug!("fresh reconnection initiated");
        }
        Err(e) => {
            error!("failed to initiate reconnection: {:?}", e);
        }
    }
}

fn attempt_connect(
    commands: &mut Commands,
    client_addr_str: &str,
    server_addr_str: &str,
    client_id: u64,
) -> Result {
    let client_addr = parse_addr(client_addr_str).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse client address '{}': {}",
            client_addr_str,
            e
        )
    })?;
    let server_addr = parse_addr(server_addr_str).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse server address '{}': {}",
            server_addr_str,
            e
        )
    })?;

    let auth = Authentication::Manual {
        server_addr,
        client_id,
        private_key: Key::default(),
        protocol_id: 0,
    };

    let client = commands
        .spawn((
            Client::default(),
            LocalAddr(client_addr),
            PeerAddr(server_addr),
            Link::new(None),
            ReplicationReceiver::default(),
            NetcodeClient::new(auth, NetcodeConfig::default())?,
            UdpIo::default(),
        ))
        .id();

    commands.trigger(Connect { entity: client });
    Ok(())
}

// TODO: I probably want to gate this to only debug builds, there isn't anything
// useful here for users.
#[cfg(debug_assertions)]
fn debug_received_entities(
    query: Query<(
        Entity,
        &crate::systems::netcode::protocol::ReplicatedSource,
        &crate::systems::netcode::protocol::ReplicatedDest,
    )>,
    stats_query: Query<(
        Entity,
        &crate::systems::stats::Uptime,
        &crate::systems::stats::Mem,
        &crate::systems::stats::Cpu,
    )>,
    client_query: Query<Entity, With<Connected>>,
) {
    let sync_count = query.iter().count();
    let stats_count = stats_query.iter().count();
    let connected_count = client_query.iter().count();

    debug!(
        "client status: {} connected, {} sync entities, {} stats entities",
        connected_count, sync_count, stats_count
    );

    if stats_count > 0 {
        for (entity, uptime, mem, cpu) in stats_query.iter().take(1) {
            debug!(
                "stats entity {:?}: uptime={}s mem={}kb cpu={:.1}%",
                entity, **uptime, **mem, **cpu
            );
        }
    }

    if sync_count > 0 {
        for (entity, source, dest) in query.iter().take(3) {
            debug!(
                "sync entity {:?}: {} -> {}",
                entity,
                source.0.display(),
                dest.0.display()
            );
        }
    }
}
