use bevy::prelude::*;
use bevy_tokio_tasks::TokioTasksRuntime;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::core::{Session, connect};

pub struct Pool;

impl Plugin for Pool {
    fn build(&self, app: &mut App) {
        app.insert_resource(Registry::default()).add_systems(
            Update,
            (
                establish_requested_connections,
                check_connection_establishment,
                cleanup_unused_connections.run_if(run_every_n_seconds(30.0)),
            ),
        );
    }
}

#[derive(Resource, Default)]
pub struct Registry {
    connections: Arc<Mutex<HashMap<String, Shared>>>,
    pending: Arc<Mutex<HashMap<String, Task>>>,
}

#[derive(Clone)]
pub struct Shared {
    pub session: Session,
    pub reference_count: usize,
    pub last_used: std::time::Instant,
}

pub struct Task {
    pub result: Arc<Mutex<Option<Result<Session, String>>>>,
    pub requesters: Vec<Entity>,
}

#[derive(Component)]
pub struct Request {
    pub host_spec: String, // user@host format TODO: port and unit tests, lightyear updated to 0.17 halfway through this hack so I'm icing what I have and committing the evil
}

#[derive(Component)]
pub struct Pending {
    pub host_spec: String,
}

#[derive(Component, Clone)]
pub struct Ref {
    pub host_spec: String,
    pub session: Session,
}

impl Registry {
    pub fn request_connection(&self, host_spec: String, entity: Entity) -> (bool, Option<Session>) {
        let mut connections = self.connections.lock().unwrap();

        if let Some(conn) = connections.get_mut(&host_spec) {
            conn.reference_count += 1;
            conn.last_used = std::time::Instant::now();
            return (true, Some(conn.session.clone()));
        }

        drop(connections);

        let mut pending = self.pending.lock().unwrap();
        if let Some(task) = pending.get_mut(&host_spec) {
            task.requesters.push(entity);
            (false, None)
        } else {
            let result = Arc::new(Mutex::new(None));
            pending.insert(
                host_spec,
                Task {
                    result: result.clone(),
                    requesters: vec![entity],
                },
            );
            (false, None)
        }
    }

    pub fn get_connection(&self, host_spec: &str) -> Option<Session> {
        let mut connections = self.connections.lock().unwrap();

        if let Some(conn) = connections.get_mut(host_spec) {
            conn.last_used = std::time::Instant::now();
            Some(conn.session.clone())
        } else {
            None
        }
    }

    pub fn release_connection(&self, host_spec: &str) {
        let mut connections = self.connections.lock().unwrap();

        if let Some(conn) = connections.get_mut(host_spec) {
            conn.reference_count = conn.reference_count.saturating_sub(1);
        }
    }

    fn add_connection(&self, host_spec: String, session: Session, initial_refs: usize) {
        let mut connections = self.connections.lock().unwrap();

        connections.insert(
            host_spec,
            Shared {
                session,
                reference_count: initial_refs,
                last_used: std::time::Instant::now(),
            },
        );
    }

    // This stuff might not be needed - pending creation is handled in request_connection future mitch cleanup task

    fn remove_pending(&self, host_spec: &str) -> Option<Task> {
        let mut pending = self.pending.lock().unwrap();
        pending.remove(host_spec)
    }

    fn cleanup_unused(&self, max_idle_time: std::time::Duration) {
        let mut connections = self.connections.lock().unwrap();
        let now = std::time::Instant::now();

        connections.retain(|host_spec, conn| {
            let should_keep =
                conn.reference_count > 0 || (now.duration_since(conn.last_used) < max_idle_time);

            if !should_keep {
                debug!("cleaning up unused ssh connection to {}", host_spec);
            }

            should_keep
        });
    }
}

fn establish_requested_connections(
    mut commands: Commands,
    runtime: ResMut<TokioTasksRuntime>,
    registry: ResMut<Registry>,
    query: Query<(Entity, &Request), (Without<Ref>, Without<Pending>)>,
) -> Result {
    for (entity, request) in &query {
        let host_spec = request.host_spec.clone();

        let (connection_ready, session_opt) =
            registry.request_connection(host_spec.clone(), entity);

        if connection_ready {
            if let Some(session) = session_opt {
                debug!(
                    "reusing existing ssh connection to {} for entity {:?}",
                    host_spec, entity
                );
                commands
                    .entity(entity)
                    .remove::<Request>()
                    .insert(Ref { host_spec, session });
            }
        } else {
            debug!(
                "Entity {:?} pending ssh connection to {}",
                entity, host_spec
            );
            commands.entity(entity).remove::<Request>().insert(Pending {
                host_spec: host_spec.clone(),
            });
            let should_start_connection = {
                let pending = registry.pending.lock().unwrap();
                if let Some(task) = pending.get(&host_spec) {
                    task.requesters.len() == 1 && task.requesters[0] == entity
                } else {
                    false
                }
            };

            if should_start_connection {
                debug!(
                    "starting new ssh connection to {} for entity {:?}",
                    host_spec, entity
                );

                // TODO: unit tests and function this hack af crap
                let (user, hostname) = if let Some((u, h)) = host_spec.split_once('@') {
                    (u.to_string(), h.to_string())
                } else {
                    let user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
                    (user, host_spec.clone())
                };

                let result = {
                    let pending = registry.pending.lock().unwrap();
                    pending.get(&host_spec).map(|task| task.result.clone())
                };

                if let Some(result_handle) = result {
                    runtime.spawn_background_task(move |_ctx| async move {
                        let res = connect(hostname, user, None)
                            .await
                            .map_err(|e| e.to_string());

                        if let Ok(mut guard) = result_handle.lock() {
                            *guard = Some(res);
                        }
                    });
                }
            }
        }
    }
    Ok(())
}

fn check_connection_establishment(
    mut commands: Commands,
    registry: ResMut<Registry>,
    pending_query: Query<&Pending>,
) -> Result {
    let pending_hosts: Vec<String> = {
        let pending = registry.pending.lock().unwrap();
        pending.keys().cloned().collect()
    };

    for host_spec in pending_hosts {
        if let Some(task) = registry.remove_pending(&host_spec) {
            let connection_ready = if let Ok(mut guard) = task.result.try_lock() {
                guard.take()
            } else {
                None
            };

            if let Some(result) = connection_ready {
                match result {
                    Ok(session) => {
                        debug!("ssh connection to {} established", host_spec);

                        let requester_count = task.requesters.len();
                        registry.add_connection(
                            host_spec.clone(),
                            session.clone(),
                            requester_count,
                        );

                        for entity in task.requesters {
                            if pending_query.get(entity).is_ok() {
                                commands.entity(entity).remove::<Pending>().insert(Ref {
                                    host_spec: host_spec.clone(),
                                    session: session.clone(),
                                });
                            }
                        }
                    }
                    Err(e) => {
                        error!("ssh connection to {} failed: {}", host_spec, e);

                        for entity in task.requesters {
                            if pending_query.get(entity).is_ok() {
                                commands.entity(entity).remove::<Pending>();
                                // TODO: Add SyncFailed component or similar here too future mitch, past jerk mitch cares not
                            }
                        }
                    }
                }
            } else {
                let mut pending = registry.pending.lock().unwrap();
                pending.insert(host_spec, task);
            }
        }
    }
    Ok(())
}

fn cleanup_unused_connections(registry: ResMut<Registry>) -> Result {
    let max_idle = std::time::Duration::from_secs(300); // 5 minutes
    registry.cleanup_unused(max_idle);
    Ok(())
}

pub fn release_connection_on_despawn(
    mut removed: RemovedComponents<Ref>,
    _registry: ResMut<Registry>,
) {
    for _entity in removed.read() {
        // Need to make Registry stuff a Component that I can share between connections and forwards, future me task
        debug!("Entity with ssh connection reference was despawned");
    }
}

fn run_every_n_seconds(seconds: f32) -> impl FnMut() -> bool {
    let mut timer = Timer::from_seconds(seconds, TimerMode::Repeating);
    move || {
        timer.tick(std::time::Duration::from_secs_f32(1.0 / 60.0)); // Assume 60 FPS
        timer.just_finished()
    }
}

impl Ref {
    pub fn session(&self) -> &Session {
        &self.session
    }
}

pub fn cleanup_ssh_ref_on_despawn(
    mut commands: Commands,
    registry: ResMut<Registry>,
    query: Query<(Entity, &Ref)>,
) {
    for (entity, ssh_ref) in &query {
        registry.release_connection(&ssh_ref.host_spec);
        commands.entity(entity).remove::<Ref>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::App;

    fn setup_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);
        app.add_plugins(bevy_tokio_tasks::TokioTasksPlugin::default());
        // Note: We can't add the actual Pool plugin because it requires real ssh connections
        // Instead, we'll add the resource and test the logic directly to test just that.
        //
        // Bevy ECS testing is nice af for this separation of "do crap" vs "decide to do crap"
        //
        // These tests are the latter, former I'm not sure how I can unit test ssh connections.
        //
        // Think something more high level outside of rust is in order for that
        // as integration tests.
        app.insert_resource(Registry::default());
        app
    }

    #[test]
    fn test_connection_registry_new_request() {
        let registry = Registry::default();
        let entity = Entity::from_bits(1);
        let host = "test@localhost".to_string();

        let (ready, session) = registry.request_connection(host.clone(), entity);

        assert!(!ready, "New connection should not be ready immediately");
        assert!(session.is_none(), "New connection should have no session");

        let pending = registry.pending.lock().unwrap();
        assert!(
            pending.contains_key(&host),
            "Pending entry should exist for host"
        );
        assert_eq!(
            pending.get(&host).unwrap().requesters.len(),
            1,
            "Should have just one requester after connection"
        );
    }

    #[test]
    fn test_connection_registry_multiple_requesters() {
        let registry = Registry::default();
        let entity1 = Entity::from_bits(1);
        let entity2 = Entity::from_bits(2);
        let host = "test@localhost".to_string();

        let (ready1, _) = registry.request_connection(host.clone(), entity1);
        assert!(!ready1, "First request should not be ready");

        let (ready2, _) = registry.request_connection(host.clone(), entity2);
        assert!(!ready2, "Second request should not be ready");

        let pending = registry.pending.lock().unwrap();
        let task = pending.get(&host).unwrap();
        assert_eq!(
            task.requesters.len(),
            2,
            "Should have exactly two requesters for same host"
        );
        assert!(task.requesters.contains(&entity1));
        assert!(task.requesters.contains(&entity2));
    }

    #[test]
    fn test_connection_registry_cleanup() {
        let registry = Registry::default();

        let max_idle = std::time::Duration::from_secs(0);

        registry.cleanup_unused(max_idle);

        let connections = registry.connections.lock().unwrap();
        assert_eq!(
            connections.len(),
            0,
            "All unused connections should be cleaned up"
        );
    }

    #[test]
    fn test_marker_component_prevents_duplicate_processing() {
        let mut app = setup_test_app();

        let entity1 = app
            .world_mut()
            .spawn(Request {
                host_spec: "test@localhost".to_string(),
            })
            .id();

        let entity2 = app
            .world_mut()
            .spawn((
                Request {
                    host_spec: "test@localhost".to_string(),
                },
                Pending {
                    host_spec: "test@localhost".to_string(),
                },
            ))
            .id();

        let query_result: Vec<Entity> = app
            .world_mut()
            .query_filtered::<Entity, (With<Request>, Without<Pending>)>()
            .iter(app.world())
            .collect();

        assert_eq!(
            query_result.len(),
            1,
            "Should only find entity1 without pending marker"
        );
        assert!(query_result.contains(&entity1), "Should find entity1");
        assert!(
            !query_result.contains(&entity2),
            "Should not find entity2 with pending marker"
        );
    }
}
