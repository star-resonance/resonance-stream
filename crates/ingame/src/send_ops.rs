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
