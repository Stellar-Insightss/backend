// Compile the production webhook modules as one integration target so their
// private dispatcher/database tests can run independently of unrelated lib
// unit tests. Authentication, validation, encryption and broadcast types come
// from the actual backend library; no alternate delivery implementation is used.
pub use stellar_analysis_backend::{auth, auth_middleware, broadcast, crypto, validation};

#[path = "../src/api/webhooks.rs"]
pub mod webhook_api;
#[path = "../src/services/webhook_dispatcher.rs"]
pub mod webhook_dispatcher;
#[path = "../src/services/webhook_event_service.rs"]
pub mod webhook_event_service;
#[path = "../src/webhooks/mod.rs"]
pub mod webhooks;
