use crate::systems::netcode::core::*;
use crate::systems::netcode::protocol::*;
use bevy::prelude::*;
use core::net::SocketAddr;
use lightyear::netcode::*;
use lightyear::prelude::server::*;
use lightyear::prelude::*;

pub struct LightYearServerPlugin;

#[derive(Component, Debug, Clone)]
pub struct ServerAddr(pub String);

#[derive(Component, Debug)]
pub struct ServerConfig;

// TODO: Keep Bundles or move to 0.15 bevy required components? They seem a bit
// jank to use ngl.
#[derive(Bundle)]
pub struct ServerConfigBundle {
    marker: ServerConfig,
    server_addr: ServerAddr,
}

impl Plugin for LightYearServerPlugin {
    fn build(&self, app: &mut App) {
        assert!(app.is_plugin_added::<bevy_cronjob::CronJobPlugin>());

        app.add_plugins(ProtocolPlugin);

        app.add_plugins(ServerPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ),
        });

        app.add_systems(Startup, (spawn_server_config, start_server).chain());
        app.add_observer(handle_new_client);
        app.add_systems(
            Update,
            (
                sync_entities_to_replicated,
                update_completion_time,
                update_stats,
                despawn_simplecopies.run_if(bevy_cronjob::schedule_passed("every 1 minute")),
            ),
        );

        // There is no need for this system unless I'm debugging, even then its sus ngl
        // Probably better for an event/message when something replicated gets added?
        // TODO: future mitch learn 0.17 bevy's event/message system and maybe fixme
        #[cfg(debug_assertions)]
        app.add_systems(
            Update,
            // These things aren't really needed to run often.
            debug_replicated_entities.run_if(bevy_cronjob::schedule_passed("every 1 minute")),
        );
    }
}

fn spawn_server_config(mut commands: Commands) {
    commands.spawn(ServerConfigBundle {
        marker: ServerConfig,
        server_addr: ServerAddr("localhost:5000".to_string()),
    });
}

// TODO: unit tests, but I kinda want to get to actually building real logic for
// this dam app I brained up in January. The side project of learning Bevy and
// other nonsense is getting old.
fn parse_addr(addr_str: &str) -> std::result::Result<SocketAddr, Box<dyn std::error::Error>> {
    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
        return Ok(addr);
    }

    use std::net::ToSocketAddrs;
    let mut addrs = addr_str.to_socket_addrs()?;
    addrs
        .next()
        .ok_or_else(|| format!("could not resolve: {}", addr_str).into())
}

fn start_server(
    mut commands: Commands,
    config_query: Query<&ServerAddr, With<ServerConfig>>,
) -> bevy::prelude::Result {
    let Ok(server_addr_str) = config_query.single() else {
        error!("server configuration not found");
        return Err(anyhow::anyhow!("server configuration not found").into());
    };

    let server_addr = parse_addr(&server_addr_str.0).map_err(|e| {
        anyhow::anyhow!(
            "failed to parse server address '{}': {}",
            server_addr_str.0,
            e
        )
    })?;

    info!(
        "lightyear udp starting on {} dbg: resolved to {}",
        server_addr_str.0, server_addr
    );

    let server = commands
        .spawn((
            NetcodeServer::new(NetcodeConfig::default()),
            LocalAddr(server_addr),
            ServerUdpIo::default(),
        ))
        .id();

    commands.trigger(Start { entity: server });
    Ok(())
}

fn handle_new_client(trigger: On<Add, Connected>, mut commands: Commands) {
    info!("client connect(): {:?}", trigger.entity);

    commands
        .entity(trigger.entity)
        .insert(ReplicationSender::new(
            SERVER_REPLICATION_INTERVAL,
            SendUpdatesMode::SinceLastAck,
            false,
        ));
}

// For right now only syncing the SimpleCopy entities as an MVP/POC. This is all
// throw away code... I think. Maybe actually having a one shot kinda thing like
// scp/cp makes sense to have as a feature? I still need to work on all the file
// i/o and metadata nonsense.
//
// Those replicated entities represent the shared "state" about sync/daemon status.
//
// Clients such as yeet monitor can then use those components to display progress/stats.
fn sync_entities_to_replicated(
    mut commands: Commands,
    // Query for entities with local components but without replicated components
    query: Query<
        (
            Entity,
            &crate::Source,
            &crate::Dest,
            &crate::Uuid,
            Option<&crate::SimpleCopy>,
            Option<&crate::SyncComplete>,
        ),
        Without<ReplicatedSource>,
    >,
) {
    for (entity, source, dest, uuid, simplecopy, complete) in query.iter() {
        debug!(
            "replicating sync entity: {} -> {}",
            source.display(),
            dest.display()
        );

        let mut entity_commands = commands.entity(entity);
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        entity_commands.insert((
            ReplicatedSource(source.0.clone()),
            ReplicatedDest(dest.0.clone()),
            ReplicatedUuid(**uuid),
            ReplicatedSyncStartTime {
                started_secs: now_secs,
            },
            Replicate::to_clients(NetworkTarget::All),
        ));

        if simplecopy.is_some() {
            entity_commands.insert(ReplicatedSimpleCopy);
        }

        if complete.is_some() {
            entity_commands.insert((
                ReplicatedSyncComplete,
                ReplicatedCompletionTime {
                    completed_secs: now_secs,
                },
            ));
        }
    }
}

// TODO: I need to add this to the monitor output at some point.
fn update_completion_time(
    mut commands: Commands,
    query: Query<Entity, (Added<crate::SyncComplete>, With<ReplicatedSource>)>,
) {
    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    for entity in query.iter() {
        commands.entity(entity).insert((
            ReplicatedSyncComplete,
            ReplicatedCompletionTime {
                completed_secs: now_secs,
            },
        ));
    }
}

// Server just updates the ReplicatedBlah from Blah components, lightyear
// handles the replication to clients.
fn update_stats(
    mut commands: Commands,
    stats_query: Query<
        (
            &crate::systems::stats::Uptime,
            &crate::systems::stats::Mem,
            &crate::systems::stats::Cpu,
        ),
        With<crate::systems::stats::PidStats>,
    >,
    clients: Query<Entity, With<Connected>>,
    existing_stats: Query<Entity, (With<crate::systems::stats::Uptime>, With<Replicate>)>,
) {
    if let Ok((uptime, mem, cpu)) = stats_query.single() {
        if existing_stats.is_empty() && !clients.is_empty() {
            trace!("spawning fresh stats entity for replication");
            commands.spawn((
                uptime.clone(),
                mem.clone(),
                cpu.clone(),
                Replicate::to_clients(NetworkTarget::All),
            ));
        } else if let Ok(stats_entity) = existing_stats.single() {
            trace!(
                "updating existing entity for replication {:?}",
                stats_entity
            );
            commands
                .entity(stats_entity)
                .insert((uptime.clone(), mem.clone(), cpu.clone()));
        }
    }
}

//TODO: keep this hack af system??????? thinking I gate this to non release
// builds its me being lazy at adding unit tests ngl.
#[cfg(debug_assertions)]
fn debug_replicated_entities(
    query: Query<(Entity, &ReplicatedSource, &ReplicatedDest), With<Replicate>>,
    stats_query: Query<Entity, (With<crate::systems::stats::Uptime>, With<Replicate>)>,
) {
    let sync_count = query.iter().count();
    let stats_count = stats_query.iter().count();

    if sync_count > 0 || stats_count > 0 {
        trace!(
            "server has {} replicated sync entities and {} replicated stats entities",
            sync_count, stats_count
        );

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

/// Cleanup system that despawns SimpleCopy sync entities older than 1 minute
/// For now runs every minute, not a huge problem to have at most 2 minutes of
/// Entities lying around in memory.
///
/// Don't keep these around for too long, TODO: is to figure out a timeframe
/// that makes sense For now purging at a minute tops cause still in make it
/// work mode. Make it right can come later. Need to learn more about lightyear
/// and bevy if I'm honest before I can figure out the "right" way to do this.
/// Still throwing spaget boxes at the wall learning how to bolt this stuff
/// together. I'm sure I'll learn better ways but that is a future mitch problem.
fn despawn_simplecopies(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            &ReplicatedSyncStartTime,
            &ReplicatedSource,
            &ReplicatedDest,
        ),
        (With<crate::SimpleCopy>, With<ReplicatedSyncComplete>),
    >,
) {
    if let Ok(duration) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        let now = duration.as_secs();

        let mut cleaned_count = 0;

        for (entity, start_time, source, dest) in query.iter() {
            let too_dam_old = now.saturating_sub(start_time.started_secs);

            // TODO: wtf makes sense? 60 seconds is low enough to see it in the
            // monitor for now. Maybe make it configurable in the ecs itself and add
            // a query for that? Actually I really should make a Config entity that
            // would be wizard to be able to modify config on the fly. And I picked
            // bevy for a reason, and that ability to dynamically change anything at
            // runtime is one of the main ones. I bought into this ecs I might as
            // well use the whole dam ecs....
            if too_dam_old > 60 {
                debug!(
                    "despawn SimpleCopy due to age: {}: {} -> {}",
                    humantime::format_duration(std::time::Duration::from_secs(too_dam_old)),
                    source.0.display(),
                    dest.0.display()
                );
                commands.entity(entity).despawn();
                cleaned_count += 1;
            }
        }

        if cleaned_count > 0 {
            let plural = if cleaned_count > 1 {
                "entities"
            } else {
                "entity"
            };
            info!(
                "despawned {} SimpleCopy sync {plural} due to age",
                cleaned_count
            );
        }
    }
}
