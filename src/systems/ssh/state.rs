use bevy::prelude::*;
use std::time::Instant;

use super::core::Session;

/// Marker component for russh ssh connection using entities
#[derive(Component)]
pub struct ConnectionEntity;

/// Host connection spec, aka user@host:port (note port nyi cause lazy)
#[derive(Component, Clone, PartialEq, Eq, Hash)]
pub struct HostSpec(pub String);

/// The actual ssh session handle (cloneable Arc-wrapped for abuse in the ECS)
#[derive(Component, Clone)]
pub struct SessionHandle(pub Session);

/// Reference count for how many sync entities are using this specific ssh connection. Will be abused in future for retries/reconnects.
#[derive(Component)]
pub struct ConnectionRefCount(pub usize);

/// Last time this connection was used, also for future retry logic
#[derive(Component)]
pub struct ConnectionLastUsed(pub Instant);

/// Marker for connections that are being established, to prevent trying to establish more than one ssh connection which is silly
#[derive(Component)]
pub struct ConnectionEstablishing;

/// Wrapper Error struct also for retry logic/display in a monitor/status command etc...
#[derive(Component)]
pub struct ConnectionError(pub String);

/// Marker component for ssh port forwarding entities
#[derive(Component)]
pub struct ForwardingEntity;

/// Underlying ssh connection this forwarding component is tied to
#[derive(Component)]
pub struct ForwardingConnectionEntity(pub Entity);

/// Remote port being forwarded locally
#[derive(Component, Clone, Copy)]
pub struct ForwardingRemotePort(pub u16);

/// Local port that was bound for use
#[derive(Component, Clone, Copy)]
pub struct ForwardingLocalPort(pub u16);

/// Reference count for how many sync entities are using this forwarding
#[derive(Component)]
pub struct ForwardingRefCount(pub usize);

/// Last time this specific forwarding was used
#[derive(Component)]
pub struct ForwardingLastUsed(pub Instant);

/// Marker for forwarding that is being established also to prevent having multiple forwards using the same ssh connection
#[derive(Component)]
pub struct ForwardingEstablishing;

/// Error information if forwarding failed also more a future thing
#[derive(Component)]
pub struct ForwardingError(pub String);

/// Reference from a sync entity to its ssh connection entity
#[derive(Component)]
pub struct SyncConnectionRef(pub Entity);

// TODO: should I make reference stuff shared kinda like future retry logic?
/// Reference from a sync entity to its ssh forwarding entity
#[derive(Component)]
pub struct SyncForwardingRef(pub Entity);

impl HostSpec {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for HostSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Debug for HostSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HostSpec({})", self.0)
    }
}

#[derive(Bundle)]
pub struct ConnectionBundle {
    pub marker: ConnectionEntity,
    pub host_spec: HostSpec,
    pub ref_count: ConnectionRefCount,
    pub last_used: ConnectionLastUsed,
    pub establishing: ConnectionEstablishing,
}

impl ConnectionBundle {
    pub fn new(host_spec: String) -> Self {
        Self {
            marker: ConnectionEntity,
            host_spec: HostSpec(host_spec),
            ref_count: ConnectionRefCount(1),
            last_used: ConnectionLastUsed(Instant::now()),
            establishing: ConnectionEstablishing,
        }
    }
}

#[derive(Bundle)]
pub struct ForwardingBundle {
    pub marker: ForwardingEntity,
    pub connection: ForwardingConnectionEntity,
    pub remote_port: ForwardingRemotePort,
    pub ref_count: ForwardingRefCount,
    pub last_used: ForwardingLastUsed,
    pub establishing: ForwardingEstablishing,
}

impl ForwardingBundle {
    pub fn new(connection_entity: Entity, remote_port: u16) -> Self {
        Self {
            marker: ForwardingEntity,
            connection: ForwardingConnectionEntity(connection_entity),
            remote_port: ForwardingRemotePort(remote_port),
            ref_count: ForwardingRefCount(1),
            last_used: ForwardingLastUsed(Instant::now()),
            establishing: ForwardingEstablishing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::app::App;

    #[test]
    fn test_ssh_connection_entity_creation() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);

        let conn_entity = app
            .world_mut()
            .spawn(ConnectionBundle::new("test@localhost".to_string()))
            .id();

        let world = app.world();
        assert!(world.get::<ConnectionEntity>(conn_entity).is_some());
        assert!(world.get::<HostSpec>(conn_entity).is_some());
        assert!(world.get::<ConnectionRefCount>(conn_entity).is_some());
        assert!(world.get::<ConnectionLastUsed>(conn_entity).is_some());
        assert!(world.get::<ConnectionEstablishing>(conn_entity).is_some());

        let host_spec = world.get::<HostSpec>(conn_entity).unwrap();
        assert_eq!(host_spec.0, "test@localhost");

        let ref_count = world.get::<ConnectionRefCount>(conn_entity).unwrap();
        assert_eq!(ref_count.0, 1);
    }

    #[test]
    fn test_ssh_forwarding_entity_creation() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);

        let conn_entity = app
            .world_mut()
            .spawn(ConnectionBundle::new("test@localhost".to_string()))
            .id();

        let fwd_entity = app
            .world_mut()
            .spawn(ForwardingBundle::new(conn_entity, 50051))
            .id();

        let world = app.world();
        assert!(world.get::<ForwardingEntity>(fwd_entity).is_some());
        assert!(
            world
                .get::<ForwardingConnectionEntity>(fwd_entity)
                .is_some()
        );
        assert!(world.get::<ForwardingRemotePort>(fwd_entity).is_some());
        assert!(world.get::<ForwardingRefCount>(fwd_entity).is_some());
        assert!(world.get::<ForwardingLastUsed>(fwd_entity).is_some());
        assert!(world.get::<ForwardingEstablishing>(fwd_entity).is_some());

        let conn_ref = world.get::<ForwardingConnectionEntity>(fwd_entity).unwrap();
        assert_eq!(conn_ref.0, conn_entity);

        let remote_port = world.get::<ForwardingRemotePort>(fwd_entity).unwrap();
        assert_eq!(remote_port.0, 50051);
    }

    #[test]
    fn test_query_connections_by_host() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);

        app.world_mut()
            .spawn(ConnectionBundle::new("host1@example.com".to_string()));
        app.world_mut()
            .spawn(ConnectionBundle::new("host2@example.com".to_string()));
        app.world_mut()
            .spawn(ConnectionBundle::new("host1@example.com".to_string())); // Duplicate

        let host1_count = app
            .world_mut()
            .query_filtered::<Entity, (With<ConnectionEntity>, With<HostSpec>)>()
            .iter(app.world())
            .filter(|&entity| {
                let host_spec = app.world().get::<HostSpec>(entity).unwrap();
                host_spec.0 == "host1@example.com"
            })
            .count();

        assert_eq!(host1_count, 2);

        let host2_count = app
            .world_mut()
            .query_filtered::<Entity, (With<ConnectionEntity>, With<HostSpec>)>()
            .iter(app.world())
            .filter(|&entity| {
                let host_spec = app.world().get::<HostSpec>(entity).unwrap();
                host_spec.0 == "host2@example.com"
            })
            .count();

        assert_eq!(host2_count, 1);
    }

    #[test]
    fn test_query_forwarding_by_port() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);

        let conn_entity = app
            .world_mut()
            .spawn(ConnectionBundle::new("test@localhost".to_string()))
            .id();

        app.world_mut()
            .spawn(ForwardingBundle::new(conn_entity, 50051));
        app.world_mut()
            .spawn(ForwardingBundle::new(conn_entity, 50052));
        app.world_mut()
            .spawn(ForwardingBundle::new(conn_entity, 50051)); // Duplicate port

        let port_50051_count = app
            .world_mut()
            .query_filtered::<Entity, (With<ForwardingEntity>, With<ForwardingRemotePort>)>()
            .iter(app.world())
            .filter(|&entity| {
                let remote_port = app.world().get::<ForwardingRemotePort>(entity).unwrap();
                remote_port.0 == 50051
            })
            .count();

        assert_eq!(port_50051_count, 2);
    }

    #[test]
    fn test_connection_lifecycle_markers() {
        let mut app = App::new();
        app.add_plugins(bevy::MinimalPlugins);

        let conn_entity = app
            .world_mut()
            .spawn(ConnectionBundle::new("test@localhost".to_string()))
            .id();

        assert!(
            app.world()
                .get::<ConnectionEstablishing>(conn_entity)
                .is_some()
        );

        app.world_mut()
            .entity_mut(conn_entity)
            .remove::<ConnectionEstablishing>();

        assert!(
            app.world()
                .get::<ConnectionEstablishing>(conn_entity)
                .is_none()
        );
    }
}
