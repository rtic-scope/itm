use super::{
    Decoder, DecoderError, DecoderErrorInt, MalformedPacket, TimestampDataRelation, TracePacket,
};

use std::io::Read;
use std::time::Duration;

pub use crate::cortex_m::LocalTimestampOptions;

/// Iterator that yield [`TracePacket`](TracePacket).
pub struct Singles<R>
where
    R: Read,
{
    decoder: Decoder<R>,
}

impl<R> Singles<R>
where
    R: Read,
{
    pub(super) fn new(decoder: Decoder<R>) -> Self {
        Self { decoder }
    }
}

impl<R> Iterator for Singles<R>
where
    R: Read,
{
    type Item = Result<TracePacket, DecoderError>;

    fn next(&mut self) -> Option<Self::Item> {
        let trace = self.decoder.next_single();

        match trace {
            Err(DecoderErrorInt::Eof) => None,
            Err(DecoderErrorInt::Io(io)) => Some(Err(DecoderError::Io(io))),
            Err(DecoderErrorInt::MalformedPacket(m)) => Some(Err(DecoderError::MalformedPacket(m))),
            Ok(trace) => Some(Ok(trace)),
        }
    }
}

/// [`Timestamps`](Timestamps) configuration.
#[derive(Clone)]
pub struct TimestampsConfiguration {
    /// Frequency of the ITM timestamp clock. Necessary to calculate a
    /// relative timestamp from global and local timestamp packets.
    pub clock_frequency: u32,

    /// Prescaler used for the ITM timestamp clock. Necessary to
    /// calculate a relative timestamp from global and local timestamp
    /// packets.
    pub lts_prescaler: LocalTimestampOptions,

    /// When set, pushes [`MalformedPacket`](MalformedPacket)s to
    /// [`TimestampedTracePackets::malformed_packets`](TimestampedTracePackets::malformed_packets)
    /// instead of returning it as an `Result::Err`.
    pub expect_malformed: bool,
}

/// A set of timestamped [`TracePacket`](TracePacket)s.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TimestampedTracePackets {
    /// Timestamp of [`packets`](Self::packets) and
    /// [`malformed_packets`](Self::malformed_packets).
    pub timestamp: Timestamp,

    /// Packets that the target generated during
    /// [`timestamp`](Self::timestamp).
    pub packets: Vec<TracePacket>,

    /// Malformed packets that the target generated during
    /// [`timestamp`](Self::timestamp).
    pub malformed_packets: Vec<MalformedPacket>,

    /// The number of [`TracePacket`](TracePacket)s consumed to generate
    /// this structure.
    pub consumed_packets: usize,
}

/// Timestamp relative to trace clock start with quality
/// descriptions. In order of decreasing quality:
/// - [`Sync`](Timestamp::Sync);
/// - [`UnknownDelay`](Timestamp::UnknownDelay);
/// - [`AssocEventDelay`](Timestamp::AssocEventDelay);
/// - [`UnknownAssocEventDelay`](Timestamp::UnknownAssocEventDelay).
///
/// A decrease in timestamp quality indicates an insufficient
/// exfiltration rate of trace packets. A decrease in timestamp
/// quality may herald an overflow event.
///
/// See also (Appendix D4.2.4).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Timestamp {
    /// The timestamp is synchronous to the ITM/DWT data and event.
    Sync(Duration),
    /// The synchronous timestamp to the ITM/DWT data is unknown, but
    /// must be between the previous and current timestamp provided
    /// here.
    UnknownDelay {
        /// The previous timestamp.
        prev: Duration,
        /// The current timestamp.
        curr: Duration,
    },
    /// The timestamp is synchronous to the ITM/DWT data packet
    /// generation, but the packet itself was delayed relative to the
    /// corresponding event due to other trace output packets.
    AssocEventDelay(Duration),
    /// The synchronous timestamp to the ITM/DWT data is unknown and
    /// the generation of the associated packet itself was delayed
    /// relative to the corresponding event due to other trace output
    /// packets. This is a combination of
    /// [`UnknownDelay`](Timestamp::UnknownDelay) and
    /// [`AssocEventDelay`](Timestamp::AssocEventDelay); the packet
    /// was generated, and the associated event occured some time
    /// between the previous and current timestamp provided here.
    UnknownAssocEventDelay {
        /// The previous timestamp.
        prev: Duration,
        /// The current timestamp.
        curr: Duration,
    },
}

/// Iterator that yield [`TimestampedTracePackets`](TimestampedTracePackets).
pub struct Timestamps<R>
where
    R: Read,
{
    decoder: Decoder<R>,
    options: TimestampsConfiguration,
    current_offset: Duration,
    gts: Gts,
    prev_lts: Duration,
}

#[cfg_attr(test, derive(Clone, Debug))]
struct Gts {
    pub lower: Option<u64>,
    pub upper: Option<u64>,
}
impl Gts {
    const GTS2_SHIFT: u32 = 26; // see (Appendix D4.2.5).

    pub fn replace_lower(&mut self, new: u64) {
        self.lower = match self.lower {
            None => Some(new),
            Some(old) => {
                let shift = 64 - new.leading_zeros();
                Some(((old >> shift) << shift) | new)
            }
        }
    }

    pub fn reset(&mut self) {
        self.lower = None;
        self.upper = None;
    }

    pub fn merge(&self) -> Option<u64> {
        if let (Some(lower), Some(upper)) = (self.lower, self.upper) {
            Some(
                upper
                    .checked_shl(Self::GTS2_SHIFT)
                    .expect("GTS merge overflow")
                    | lower,
            )
        } else {
            None
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn from_one(gts: TracePacket) -> Self {
        match gts {
            TracePacket::GlobalTimestamp1 { ts: lower, .. } => Self {
                lower: Some(lower),
                upper: None,
            },
            TracePacket::GlobalTimestamp2 { ts: upper, .. } => Self {
                lower: None,
                upper: Some(upper),
            },
            _ => unreachable!(),
        }
    }
}

impl<R> Timestamps<R>
where
    R: Read,
{
    pub(super) fn new(decoder: Decoder<R>, options: TimestampsConfiguration) -> Self {
        if options.lts_prescaler == LocalTimestampOptions::Disabled {
            unimplemented!("Generating approximate absolute timestamps from global timestamps alone is not yet supported");
        }

        Self {
            current_offset: Duration::from_nanos(0),
            decoder,
            options,
            gts: Gts {
                lower: None,
                upper: None,
            },
            // NOTE: required because GTS resets current_offset. GTS
            // -> LTS, would yield incorrect prev timestamp if this
            // field, upon which only local timestamps are applied, is
            // not used.
            prev_lts: Duration::from_nanos(0),
        }
    }

    fn next_timestamped(
        &mut self,
        options: TimestampsConfiguration,
    ) -> Result<TimestampedTracePackets, DecoderErrorInt> {
        use std::ops::Add;

        let mut packets: Vec<TracePacket> = vec![];
        let mut malformed_packets: Vec<MalformedPacket> = vec![];
        let mut consumed_packets: usize = 0;

        fn apply_lts(
            prev_offset: &mut Duration,
            lts: u64,
            data_relation: TimestampDataRelation,
            current_offset: &mut Duration,
            options: &TimestampsConfiguration,
        ) -> Timestamp {
            let offset = calc_offset(lts, Some(options.lts_prescaler), options.clock_frequency);
            *current_offset = current_offset.add(offset);

            let lts = match data_relation {
                TimestampDataRelation::Sync => Timestamp::Sync(*current_offset),
                TimestampDataRelation::UnknownDelay => Timestamp::UnknownDelay {
                    prev: *prev_offset,
                    curr: *current_offset,
                },
                TimestampDataRelation::AssocEventDelay => {
                    Timestamp::AssocEventDelay(*current_offset)
                }
                TimestampDataRelation::UnknownAssocEventDelay => {
                    Timestamp::UnknownAssocEventDelay {
                        prev: *prev_offset,
                        curr: *current_offset,
                    }
                }
            };
            *prev_offset = *current_offset;
            lts
        }

        fn apply_gts(gts: &Gts, current_offset: &mut Duration, options: &TimestampsConfiguration) {
            if let Some(gts) = gts.merge() {
                let offset = calc_offset(gts, None, options.clock_frequency);
                *current_offset = offset;
            }
        }

        loop {
            consumed_packets += 1;
            match self.decoder.next_single() {
                Err(DecoderErrorInt::MalformedPacket(m)) if options.expect_malformed => {
                    malformed_packets.push(m);
                }
                Err(e) => return Err(e),
                Ok(packet) => match packet {
                    // A local timestamp: packets received up to this point
                    // relate to this local timestamp. Return these.
                    TracePacket::LocalTimestamp1 { ts, data_relation } => {
                        return Ok(TimestampedTracePackets {
                            timestamp: apply_lts(
                                &mut self.prev_lts,
                                ts.into(),
                                data_relation,
                                &mut self.current_offset,
                                &self.options,
                            ),
                            packets,
                            malformed_packets,
                            consumed_packets,
                        });
                    }
                    TracePacket::LocalTimestamp2 { ts } => {
                        return Ok(TimestampedTracePackets {
                            timestamp: apply_lts(
                                &mut self.prev_lts,
                                ts.into(),
                                TimestampDataRelation::Sync,
                                &mut self.current_offset,
                                &self.options,
                            ),
                            packets,
                            malformed_packets,
                            consumed_packets,
                        });
                    }

                    // A global timestamp: store until we have both the
                    // upper (GTS2) and lower (GTS1) bits.
                    TracePacket::GlobalTimestamp1 { ts, wrap, clkch } => {
                        self.gts.replace_lower(ts);

                        if wrap {
                            // upper bits have changed; GTS2 incoming
                            self.gts.upper = None;
                        } else if clkch {
                            // system has asserted clock change input; full GTS incoming
                            //
                            // A clock change signal that the system
                            // asserts if there is a change in the ratio
                            // between the global timestamp clock
                            // frequency and the processor clock
                            // frequency. Implementation and use of the
                            // clock change signal is optional and
                            // deprecated.
                            self.gts.reset();
                        } else {
                            apply_gts(&self.gts, &mut self.current_offset, &options);
                        }
                    }
                    TracePacket::GlobalTimestamp2 { ts } => {
                        self.gts.upper = Some(ts);
                        apply_gts(&self.gts, &mut self.current_offset, &options);
                    }

                    packet => packets.push(packet),
                },
            }
        }
    }
}

impl<R> Iterator for Timestamps<R>
where
    R: Read,
{
    type Item = Result<TimestampedTracePackets, DecoderError>;

    fn next(&mut self) -> Option<Self::Item> {
        let trace = self.next_timestamped(self.options.clone());

        match trace {
            Err(DecoderErrorInt::Eof) => None,
            Err(DecoderErrorInt::Io(io)) => Some(Err(DecoderError::Io(io))),
            Err(DecoderErrorInt::MalformedPacket(m)) => Some(Err(DecoderError::MalformedPacket(m))),
            Ok(trace) => Some(Ok(trace)),
        }
    }
}

fn calc_offset(ts: u64, prescaler: Option<LocalTimestampOptions>, freq: u32) -> Duration {
    let prescale = match prescaler {
        None | Some(LocalTimestampOptions::Enabled) => 1,
        Some(LocalTimestampOptions::EnabledDiv4) => 4,
        Some(LocalTimestampOptions::EnabledDiv16) => 16,
        Some(LocalTimestampOptions::EnabledDiv64) => 64,
        Some(LocalTimestampOptions::Disabled) => unreachable!(), // checked in `Timestamps::new`
    };
    let ticks = ts * prescale;
    let seconds = ticks as f64 / freq as f64;

    // NOTE(ceil) we rount up so as to not report an event before it
    // occurs on hardware.
    Duration::from_nanos((seconds * 1e9).ceil() as u64)
}

#[cfg(test)]
mod timestamp_utils {
    use super::*;

    #[test]
    fn gts() {
        let mut gts = Gts {
            lower: Some(1), // bit 1
            upper: Some(1), // bit 26
        };
        assert_eq!(gts.merge(), Some(67108865));

        gts.replace_lower(127);
        assert_eq!(gts.merge(), Some(67108991));

        let gts = Gts {
            lower: None,
            upper: None,
        };
        assert_eq!(gts.merge(), None, "noop merge");

        let mut gts = Gts {
            lower: Some(42),
            upper: Some(42),
        };
        assert_eq!(
            gts.merge(),
            Some((42 << Gts::GTS2_SHIFT) | 42),
            "(42, 42) merge"
        );

        gts.replace_lower(0b1101011);
        assert_eq!(
            gts.merge(),
            Some((42 << Gts::GTS2_SHIFT) | 0b1101011),
            "replace whole merge"
        );

        let mut gts = Gts {
            lower: Some(42),
            upper: Some(42),
        };
        gts.replace_lower(1);
        assert_eq!(
            gts.merge(),
            Some((42 << Gts::GTS2_SHIFT) | 43),
            "replace partial merge"
        );
    }

    #[test]
    fn offset() {
        assert_eq!(
            calc_offset(1000, Some(LocalTimestampOptions::EnabledDiv4), 16_000_000),
            Duration::from_micros(250),
        );
    }
}

#[cfg(test)]
mod timestamps {
    use super::Duration;
    use crate::*;

    const FREQ: u32 = 16_000_000;

    /// Check whether timestamps are correctly generated by effectively
    /// comparing `Timestamps::next_timestamps` and [outer_calc_offset].
    #[test]
    fn check_timestamps() {
        #[rustfmt::skip]
        let stream: &[u8] = &[
            // PC sample (sleeping)
            0b0001_0101,
            0b0000_0000,

            // PC sample (sleeping)
            0b0001_0101,
            0b0000_0000,

            // PC sample (sleeping)
            0b0001_0101,
            0b0000_0000,

            // GTS1
            0b1001_0100,
            0b1000_0000,
            0b1010_0000,
            0b1000_0100,
            0b0000_0000,

            // GTS2 (48-bit)
            0b1011_0100,
            0b1011_1101,
            0b1111_0100,
            0b1001_0001,
            0b0000_0001,

            // LTS1
            0b1100_0000,
            0b1100_1001,
            0b0000_0001,

            // Pull!

            // PC sample (sleeping)
            0b0001_0101,
            0b0000_0000,

            // LTS1
            0b1100_0000,
            0b1100_1001,
            0b0000_0001,

            // Pull!

            // Overflow
            0b0111_0000,

            // LTS1
            0b1100_0000,
            0b1100_1001,
            0b0000_0001,

            // Pull!

            // GTS1
            0b1001_0100,
            0b1000_0000,
            0b1010_0000,
            0b1000_0100,
            0b0000_0000,

            // GTS2 (48-bit)
            0b1011_0100,
            0b1011_1101,
            0b1111_0100,
            0b1001_0001,
            0b0000_0001,

            // LTS1
            0b1111_0000,
            0b1100_1001,
            0b0000_0001,

            // LTS2
            0b0110_0000,
        ];

        let decoder = Decoder::new(stream.clone(), DecoderOptions { ignore_eof: false });
        let mut it = decoder.timestamps(TimestampsConfiguration {
            clock_frequency: FREQ,
            lts_prescaler: LocalTimestampOptions::Enabled,
            expect_malformed: false,
        });

        for set in [
            TimestampedTracePackets {
                packets: [
                    TracePacket::PCSample { pc: None },
                    TracePacket::PCSample { pc: None },
                    TracePacket::PCSample { pc: None },
                ]
                .into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(10026857009420563)),
                consumed_packets: 6,
            },
            TimestampedTracePackets {
                packets: [TracePacket::PCSample { pc: None }].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(10026857009433126)),
                consumed_packets: 2,
            },
            TimestampedTracePackets {
                packets: [TracePacket::Overflow].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(10026857009445689)),
                consumed_packets: 2,
            },
            TimestampedTracePackets {
                packets: [].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::UnknownAssocEventDelay {
                    prev: Duration::from_nanos(10026857009445689),
                    curr: Duration::from_nanos(10026857009420563),
                },
                consumed_packets: 3,
            },
            TimestampedTracePackets {
                packets: [].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(10026857009420938)),
                consumed_packets: 1,
            },
        ]
        .iter()
        {
            assert_eq!(it.next().unwrap().unwrap(), *set);
        }
    }

    /// Test cases where a GTS2 applied to two GTS1; 64-bit GTS2; and
    /// compares timestamps to precalculated [Duration] offsets.
    #[test]
    fn gts_compression() {
        #[rustfmt::skip]
        let stream: &[u8] = &[
            // LTS2
            0b0110_0000,

            // GTS1 (bit 1 set)
            0b1001_0100,
            0b1000_0001,
            0b1000_0000,
            0b1000_0000,
            0b0000_0000,

            // GTS2 (64-bit, bit 26 set)
            0b1011_0100,
            0b1000_0001,
            0b1000_0000,
            0b1000_0000,
            0b1000_0000,
            0b1000_0000,
            0b0000_0000,

            // LTS2
            0b0110_0000,

            // GTS1 (compressed)
            0b1001_0100,
            0b1111_1111,
            0b0000_0000,

            // LTS2
            0b0110_0000,

            // TODO: add a section where a GTS1 must merge with the
            // previous GTS1
        ];

        let decoder = Decoder::new(stream.clone(), DecoderOptions { ignore_eof: false });
        let mut it = decoder.timestamps(TimestampsConfiguration {
            clock_frequency: FREQ,
            lts_prescaler: LocalTimestampOptions::Enabled,
            expect_malformed: false,
        });

        for set in [
            TimestampedTracePackets {
                packets: [].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(375)),
                consumed_packets: 1,
            },
            TimestampedTracePackets {
                packets: [].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(4194304438)),
                consumed_packets: 3,
            },
            TimestampedTracePackets {
                packets: [].into(),
                malformed_packets: [].into(),
                timestamp: Timestamp::Sync(Duration::from_nanos(4194312313)),
                consumed_packets: 2,
            },
        ]
        .iter()
        {
            let ttp = it.next().unwrap().unwrap();
            assert_eq!(ttp, *set);
        }
    }
}
