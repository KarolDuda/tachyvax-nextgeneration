//! NEOX core: pure logic with no Windows APIs. Everything that touches the system lives behind the traits in `engine`.

pub mod autotune;
pub mod catalog;
pub mod engine;
pub mod health;
pub mod activation;
pub mod license;
pub mod models;
pub mod nettune;
pub mod profiles;
pub mod state;
pub mod timefmt;
