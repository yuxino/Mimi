//! Native and network clients for live recognition and translation.

pub mod pipeline;
pub mod realtime;
pub mod recognition;
#[cfg(test)]
mod tests;
pub mod translation;
pub mod transport;
