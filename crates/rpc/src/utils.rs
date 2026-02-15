use std::{iter::StepBy, ops::RangeInclusive};

macro_rules! await_handler {
    ($h:expr) => {
        match $h.await {
            Ok(res) => res,
            Err(_) => return Err("task panicked or cancelled".to_string()),
        }
    };

    (@option $h:expr) => {
        match $h.await {
            Ok(Some(res)) => res,
            _ => return Err("task panicked or cancelled".to_string()),
        }
    };

    (@response $h:expr) => {
        match $h.await {
            Ok(res) => res,
            _ => {
                return ResponsePayload::internal_error_message(std::borrow::Cow::Borrowed(
                    "task panicked or cancelled",
                ))
            }
        }
    };

    (@response_option $h:expr) => {
        match $h.await {
            Ok(Some(res)) => res,
            _ => {
                return ResponsePayload::internal_error_message(std::borrow::Cow::Borrowed(
                    "task panicked or cancelled",
                ))
            }
        }
    };
}

pub(crate) use await_handler;

macro_rules! response_tri {
    ($h:expr) => {
        match $h {
            Ok(res) => res,
            Err(err) => return ResponsePayload::internal_error_message(err.to_string().into()),
        }
    };

    ($h:expr, $msg:literal) => {
        match $h {
            Ok(res) => res,
            Err(_) => return ResponsePayload::internal_error_message($msg.into()),
        }
    };

    ($h:expr, $obj:expr) => {
        match $h {
            Ok(res) => res,
            Err(err) => returnResponsePayload::internal_error_with_message_and_obj(
                err.to_string().into(),
                $obj,
            ),
        }
    };

    ($h:expr, $msg:literal, $obj:expr) => {
        match $h {
            Ok(res) => res,
            Err(err) => {
                return ResponsePayload::internal_error_with_message_and_obj($msg.into(), $obj)
            }
        }
    };
}

pub(crate) use response_tri;

/// An iterator that yields _inclusive_ block ranges of a given step size
#[derive(Debug)]
pub(crate) struct BlockRangeInclusiveIter {
    iter: StepBy<RangeInclusive<u64>>,
    step: u64,
    end: u64,
}

impl BlockRangeInclusiveIter {
    pub(crate) fn new(range: RangeInclusive<u64>, step: u64) -> Self {
        Self { end: *range.end(), iter: range.step_by(step as usize + 1), step }
    }
}

impl Iterator for BlockRangeInclusiveIter {
    type Item = (u64, u64);

    fn next(&mut self) -> Option<Self::Item> {
        let start = self.iter.next()?;
        let end = (start + self.step).min(self.end);
        if start > end {
            return None;
        }
        Some((start, end))
    }
}
