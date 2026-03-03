#[cfg(feature = "web")]
mod api;
#[cfg(feature = "web")]
mod server;
#[cfg(feature = "web")]
mod ws;

#[cfg(feature = "web")]
pub use server::serve;
