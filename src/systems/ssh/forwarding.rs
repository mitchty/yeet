use bevy::prelude::*;
use bevy_tokio_tasks::TokioTasksRuntime;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::core::setup_port_forward;
use super::pool::Ref as ConnectionRef;
use crate::SshForwarding;

pub struct Manager;

impl Plugin for Manager {
    fn build(&self, app: &mut App) {
        app.insert_resource(Registry::default()).add_systems(
            Update,
            (
                setup_requested_forwarding,
                check_forwarding_establishment,
                cleanup_unused_forwarding.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(60),
                )),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct Registry {
    forwarding: Arc<Mutex<HashMap<(String, u16), Shared>>>,
    pending: Arc<Mutex<HashMap<(String, u16), Task>>>,
}

#[derive(Clone)]
pub struct Shared {
    pub local_port: u16,
    pub remote_port: u16,
    pub reference_count: usize,
    pub last_used: std::time::Instant,
}

pub struct Task {
    pub result: Arc<Mutex<Option<Result<u16, String>>>>,
    pub requesters: Vec<Entity>,
}

#[derive(Component)]
pub struct Request {
    pub remote_port: u16,
}

#[derive(Component)]
pub struct Pending {
    pub host_spec: String,
    pub remote_port: u16,
}

// TODO: Maybe move more of this logic into the ecs itself, future mitch task
impl Registry {
    pub fn request_forwarding(&self, host_spec: String, remote_port: u16, entity: Entity) -> bool {
        let key = (host_spec, remote_port);
        let mut forwarding = self.forwarding.lock().unwrap();

        if let Some(fwd) = forwarding.get_mut(&key) {
            fwd.reference_count += 1;
            fwd.last_used = std::time::Instant::now();
            return true;
        }

        let mut pending = self.pending.lock().unwrap();
        if let Some(task) = pending.get_mut(&key) {
            task.requesters.push(entity);
            return false;
        }

        false
    }

    pub fn get_forwarding(&self, host_spec: &str, remote_port: u16) -> Option<u16> {
        let key = (host_spec.to_string(), remote_port);
        let mut forwarding = self.forwarding.lock().unwrap();

        if let Some(fwd) = forwarding.get_mut(&key) {
            fwd.last_used = std::time::Instant::now();
            Some(fwd.local_port)
        } else {
            None
        }
    }

    pub fn release_forwarding(&self, host_spec: &str, remote_port: u16) {
        let key = (host_spec.to_string(), remote_port);
        let mut forwarding = self.forwarding.lock().unwrap();

        if let Some(fwd) = forwarding.get_mut(&key) {
            fwd.reference_count = fwd.reference_count.saturating_sub(1);
        }
    }

    fn add_forwarding(
        &self,
        host_spec: String,
        remote_port: u16,
        local_port: u16,
        initial_refs: usize,
    ) {
        let key = (host_spec, remote_port);
        let mut forwarding = self.forwarding.lock().unwrap();

        forwarding.insert(
            key,
            Shared {
                local_port,
                remote_port,
                reference_count: initial_refs,
                last_used: std::time::Instant::now(),
            },
        );
    }

    fn start_forwarding_task(
        &self,
        host_spec: String,
        remote_port: u16,
        initial_requesters: Vec<Entity>,
    ) -> Arc<Mutex<Option<Result<u16, String>>>> {
        let key = (host_spec, remote_port);
        let result = Arc::new(Mutex::new(None));

        let mut pending = self.pending.lock().unwrap();
        pending.insert(
            key,
            Task {
                result: result.clone(),
                requesters: initial_requesters,
            },
        );

        result
    }

    fn remove_pending(&self, host_spec: &str, remote_port: u16) -> Option<Task> {
        let key = (host_spec.to_string(), remote_port);
        let mut pending = self.pending.lock().unwrap();
        pending.remove(&key)
    }

    fn cleanup_unused(&self, max_idle_time: std::time::Duration) {
        let mut forwarding = self.forwarding.lock().unwrap();
        let now = std::time::Instant::now();

        forwarding.retain(|(host_spec, remote_port), fwd| {
            let should_keep =
                fwd.reference_count > 0 || (now.duration_since(fwd.last_used) < max_idle_time);

            if !should_keep {
                debug!(
                    "cleaning up unused ssh forwarding {}:{} -> {}",
                    host_spec, remote_port, fwd.local_port
                );
            }

            should_keep
        });
    }
}

fn setup_requested_forwarding(
    mut commands: Commands,
    runtime: ResMut<TokioTasksRuntime>,
    registry: ResMut<Registry>,
    query: Query<(Entity, &ConnectionRef, &Request), (Without<SshForwarding>, Without<Pending>)>,
) -> Result {
    for (entity, ssh_ref, fwd_request) in &query {
        let host_spec = ssh_ref.host_spec.clone();
        let remote_port = fwd_request.remote_port;

        if registry.request_forwarding(host_spec.clone(), remote_port, entity) {
            if let Some(local_port) = registry.get_forwarding(&host_spec, remote_port) {
                debug!(
                    "reusing existing ssh forwarding {}:{} -> {} for entity {:?}",
                    host_spec, remote_port, local_port, entity
                );
                commands
                    .entity(entity)
                    .remove::<Request>()
                    .insert(SshForwarding {
                        local_port,
                        remote_port,
                    });
            }
            continue;
        }

        debug!(
            "entity {:?} pending ssh forwarding {}:{}",
            entity, host_spec, remote_port
        );
        commands.entity(entity).remove::<Request>().insert(Pending {
            host_spec: host_spec.clone(),
            remote_port,
        });

        let should_start_forwarding = {
            let pending = registry.pending.lock().unwrap();
            if let Some(task) = pending.get(&(host_spec.clone(), remote_port)) {
                task.requesters.len() == 1 && task.requesters[0] == entity
            } else {
                false
            }
        };

        if should_start_forwarding {
            debug!(
                "starting new ssh forwarding {}:{} for entity {:?}",
                host_spec, remote_port, entity
            );

            let result =
                registry.start_forwarding_task(host_spec.clone(), remote_port, vec![entity]);

            let session = ssh_ref.session.clone();
            let result_clone = result.clone();
            runtime.spawn_background_task(move |_ctx| async move {
                let res = setup_port_forward(session, remote_port)
                    .await
                    .map_err(|e| e.to_string());

                if let Ok(mut guard) = result_clone.lock() {
                    *guard = Some(res);
                }
            });
        }
    }
    Ok(())
}

fn check_forwarding_establishment(
    mut commands: Commands,
    registry: ResMut<Registry>,
    pending_query: Query<&Pending>,
) -> Result {
    let pending_keys: Vec<(String, u16)> = {
        let pending = registry.pending.lock().unwrap();
        pending.keys().cloned().collect()
    };

    for (host_spec, remote_port) in pending_keys {
        if let Some(task) = registry.remove_pending(&host_spec, remote_port) {
            let forwarding_ready = if let Ok(mut guard) = task.result.try_lock() {
                guard.take()
            } else {
                None
            };

            if let Some(result) = forwarding_ready {
                match result {
                    Ok(local_port) => {
                        debug!(
                            "ssh forwarding {}:{} -> {} established",
                            host_spec, remote_port, local_port
                        );

                        let requester_count = task.requesters.len();
                        registry.add_forwarding(
                            host_spec.clone(),
                            remote_port,
                            local_port,
                            requester_count,
                        );

                        for entity in task.requesters {
                            if pending_query.get(entity).is_ok() {
                                commands
                                    .entity(entity)
                                    .remove::<Pending>()
                                    .insert(SshForwarding {
                                        local_port,
                                        remote_port,
                                    });
                            }
                        }
                    }
                    Err(e) => {
                        error!("ssh forwarding {}:{} failed: {}", host_spec, remote_port, e);

                        for entity in task.requesters {
                            if pending_query.get(entity).is_ok() {
                                commands.entity(entity).remove::<Pending>();
                                // TODO: Add SyncFailed component or similar for future retry logic
                            }
                        }
                    }
                }
            } else {
                // Retry next tick/fn call I guess
                let mut pending = registry.pending.lock().unwrap();
                pending.insert((host_spec, remote_port), task);
            }
        }
    }
    Ok(())
}

fn cleanup_unused_forwarding(registry: ResMut<Registry>) -> Result {
    let max_idle = std::time::Duration::from_secs(600); // 10 minutes
    registry.cleanup_unused(max_idle);
    Ok(())
}

// TODO: learn to use events and On to do despawning vs hack lots of systems
// that run like a dumass.
pub fn cleanup_forwarding_on_despawn(
    mut commands: Commands,
    registry: ResMut<Registry>,
    query: Query<(Entity, &ConnectionRef, &SshForwarding)>,
) {
    for (entity, ssh_ref, forwarding) in &query {
        registry.release_forwarding(&ssh_ref.host_spec, forwarding.remote_port);
        commands
            .entity(entity)
            .remove::<SshForwarding>()
            .remove::<Request>();
    }
}
