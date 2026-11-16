//! `ZTcpClient.Send(ReadOnlySpan<byte>)` called through `il2cpp_runtime_invoke`, as the
//! pump's [`SendOps`]. The span's layout is the one place the ABI shows.
//!
//! NOT VERIFIED (only a run in the game settles it): that this build's `ReadOnlySpan<byte>`
//! is a pointer then an `i32` length, that it is passed by reference to `runtime_invoke`,
//! that the bytes may live outside a managed array, and what `Send` returns.

use crate::error::ProbeFailure;
use crate::log::LogSink;
use crate::pump::SendOps;
use crate::send::SendOutcome;
use crate::walk::{Arg, Il2cppOps, Walked, Walker};

/// A `ReadOnlySpan<byte>` as a 16-byte struct: a reference and an `int` length (plus
/// padding to 16).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SpanBytes {
    pub reference: *const u8,
    pub length: i32,
    pub _pad: u32,
}

impl SpanBytes {
    /// Over `bytes`, which must stay alive and unmoved while the span is used.
    pub fn over(bytes: &[u8]) -> Self {
        Self {
            reference: bytes.as_ptr(),
            length: bytes.len() as i32,
            _pad: 0,
        }
    }
}

/// [`SendOps`] over the walk: each `live_client` walks again from the singleton.
pub struct Il2cppSendOps<O, L> {
    walker: Walker<O>,
    log: L,
    walked: Option<Walked>,
}

impl<O: Il2cppOps, L: LogSink> Il2cppSendOps<O, L> {
    pub fn new(walker: Walker<O>, log: L) -> Self {
        Self {
            walker,
            log,
            walked: None,
        }
    }

    pub fn walker(&self) -> &Walker<O> {
        &self.walker
    }
}

impl<O: Il2cppOps, L: LogSink> SendOps for Il2cppSendOps<O, L> {
    fn live_client(&mut self) -> Result<usize, SendOutcome> {
        self.walked = None;
        match self.walker.walk(&mut self.log) {
            Ok(walked) => {
                self.walked = Some(walked);
                Ok(walked.client)
            }
            Err(ProbeFailure::NoClient(detail)) => Err(SendOutcome::NoClient(detail)),
            Err(ProbeFailure::Unsupported(e)) => Err(SendOutcome::Unsupported(e.detail())),
        }
    }

    fn send(&mut self, client: usize, frame: &[u8]) -> Result<(), SendOutcome> {
        let Some(walked) = self.walked else {
            return Err(SendOutcome::NoClient("no walk".into()));
        };
        let span = Arg::Span {
            ptr: frame.as_ptr() as usize,
            len: frame.len(),
        };
        // The return value is ignored: what it means is not known.
        self.walker
            .ops_mut()
            .invoke(walked.send_method, client, &[span])
            .map(|_| ())
            .map_err(|_| SendOutcome::NotSent("exception:Send".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::StepError;
    use crate::log::RotatingLog;
    use crate::send::SendOutcome;
    use crate::walk::tests::{Call, FakeOps, CLIENT_OBJ, SEND};
    use crate::walk::{Arg, Walker};
    use std::mem::{align_of, offset_of, size_of};

    #[test]
    fn the_span_is_sixteen_bytes() {
        assert_eq!(size_of::<SpanBytes>(), 16);
        assert_eq!(align_of::<SpanBytes>(), 8);
        assert_eq!(offset_of!(SpanBytes, reference), 0);
        assert_eq!(offset_of!(SpanBytes, length), 8);
        let bytes = [1u8, 2, 3];
        let span = SpanBytes::over(&bytes);
        assert_eq!(span.length, 3);
        assert_eq!(span.reference, bytes.as_ptr());
    }

    fn ops(fake: FakeOps) -> Il2cppSendOps<FakeOps, RotatingLog> {
        Il2cppSendOps::new(Walker::new(fake), RotatingLog::inert())
    }

    #[test]
    fn send_invokes_send_with_one_span_over_the_frame() {
        let mut o = ops(FakeOps::world());
        assert_eq!(o.live_client(), Ok(CLIENT_OBJ));
        let frame = [9u8, 8, 7, 6];
        assert_eq!(o.send(CLIENT_OBJ, &frame), Ok(()));
        let call = o.walker().ops().calls.last().unwrap().clone();
        assert_eq!(
            call,
            Call::Invoke(
                SEND,
                CLIENT_OBJ,
                vec![Arg::Span {
                    ptr: frame.as_ptr() as usize,
                    len: 4
                }]
            )
        );
    }

    #[test]
    fn an_exception_is_not_sent() {
        let mut fake = FakeOps::world();
        fake.results.insert(SEND, Err(()));
        let mut o = ops(fake);
        o.live_client().unwrap();
        assert_eq!(
            o.send(CLIENT_OBJ, &[1]),
            Err(SendOutcome::NotSent("exception:Send".into()))
        );
    }

    #[test]
    fn a_failing_walk_passes_through_without_invoking_send() {
        let mut fake = FakeOps::world();
        fake.memory.insert(0x5000 + 0x68, 0);
        let mut o = ops(fake);
        assert_eq!(
            o.live_client(),
            Err(SendOutcome::NoClient("client null".into()))
        );
        let sends = o
            .walker()
            .ops()
            .calls
            .iter()
            .filter(|c| matches!(c, Call::Invoke(SEND, ..)))
            .count();
        assert_eq!(sends, 0);

        let mut fake = FakeOps::world();
        fake.classes.clear();
        let mut o = ops(fake);
        assert_eq!(
            o.live_client(),
            Err(SendOutcome::Unsupported(
                StepError::class("ZRpcCtrl").detail()
            ))
        );
    }
}
