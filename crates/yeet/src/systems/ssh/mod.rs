pub mod core;
pub mod forwarding;
pub mod pool;
pub mod state;

pub use forwarding::Manager;
pub use pool::Pool;

// Export only the public side of the module with names that would make sense for client usage.
pub use core::{Client, Session, connect, setup_port_forward};
pub use forwarding::{Registry as FowardingRegistry, Request as ForwardingRequest};
pub use pool::{Ref as PoolRef, Registry as PoolRegistry, Request as PoolRequest};

// Re-export state components for bevy entities/queries
pub use state::{
    ConnectionEntity, ConnectionRefCount, ForwardingEntity, ForwardingRefCount, HostSpec,
    SessionHandle,
};
