//! Production adapters preserve network futures without observing audio.
//! No recording state, task-local metadata, decoding, files or queues exist here.
use crate::core::audio_input::AudioSource;
use std::future::Future;

pub type AudioContext = ();

pub async fn scope<F: Future>(_: AudioSource, _: u64, future: F) -> F::Output {
    future.await
}

pub fn context() -> Option<AudioContext> {
    None
}

pub async fn scope_context<F: Future>(_: Option<AudioContext>, future: F) -> F::Output {
    future.await
}

pub struct SendTicket;

impl SendTicket {
    pub async fn observe<F, E>(self, future: F) -> Result<(), E>
    where
        F: Future<Output = Result<(), E>>,
    {
        future.await
    }
}

pub fn begin_pcm(_: &[u8], _: u32) -> SendTicket {
    SendTicket
}

pub fn begin_json(_: &str, _: u32) -> SendTicket {
    SendTicket
}

pub fn begin_volcano(_: &[u8], _: u32) -> SendTicket {
    SendTicket
}
