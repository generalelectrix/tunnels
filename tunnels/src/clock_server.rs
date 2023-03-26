//! Advertise a clock bank stream over DNSSD.
//! Provide a strongly-typed receiver.
//! FIXME: would be nice to clean up deserialization to avoid so many allocations.

use zero_configure::pub_sub::PublisherService;

use crate::clock_bank::ClockBank;

pub type ClockPublisher = PublisherService<ClockBank>;
