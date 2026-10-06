//! Monotonic response-request ordering.
//!
//! Decode, resampling, response transformation, FFT planning, and allocation happen outside this
//! crate's audio callback. Their owner attaches a [`RequestId`] to each job; only the newest live id
//! may publish a prepared response. This small gate prevents an older, slower completion from
//! replacing a newer request.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

impl RequestId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestOrderError {
    Exhausted,
}

/// Issues strictly increasing ids and identifies the one completion still eligible to publish.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequestOrder {
    latest_issued: Option<RequestId>,
    live: Option<RequestId>,
}

impl RequestOrder {
    pub const fn new() -> Self {
        Self {
            latest_issued: None,
            live: None,
        }
    }

    pub fn issue(&mut self) -> Result<RequestId, RequestOrderError> {
        let raw = match self.latest_issued {
            None => 0,
            Some(id) => id.0.checked_add(1).ok_or(RequestOrderError::Exhausted)?,
        };
        let id = RequestId(raw);
        self.latest_issued = Some(id);
        self.live = Some(id);
        Ok(id)
    }

    pub const fn live(&self) -> Option<RequestId> {
        self.live
    }

    /// Whether a completed job is still the newest non-cancelled request.
    pub const fn may_publish(&self, id: RequestId) -> bool {
        matches!(self.live, Some(live) if live.0 == id.0)
    }

    /// Consume the publication right. Repeated completion cannot publish twice.
    pub fn acknowledge_publication(&mut self, id: RequestId) -> bool {
        if self.may_publish(id) {
            self.live = None;
            true
        } else {
            false
        }
    }

    /// Cancel only the named current request. Cancelling stale work cannot cancel newer work.
    pub fn cancel(&mut self, id: RequestId) -> bool {
        if self.may_publish(id) {
            self.live = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_latest_completion_can_publish() {
        let mut order = RequestOrder::new();
        let old = order.issue().unwrap();
        let new = order.issue().unwrap();
        assert!(old < new);
        assert!(!order.may_publish(old));
        assert!(order.may_publish(new));
        assert!(!order.acknowledge_publication(old));
        assert!(order.acknowledge_publication(new));
        assert!(!order.acknowledge_publication(new));
    }

    #[test]
    fn stale_cancel_cannot_abandon_newer_work() {
        let mut order = RequestOrder::new();
        let old = order.issue().unwrap();
        let new = order.issue().unwrap();
        assert!(!order.cancel(old));
        assert_eq!(order.live(), Some(new));
        assert!(order.cancel(new));
        assert_eq!(order.live(), None);

        let after_cancel = order.issue().unwrap();
        assert!(after_cancel > new);
    }
}
