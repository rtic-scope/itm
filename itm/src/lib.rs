//! # `itm`
//!
//! A decoder for the ITM and DWT packet protocol as specifed in the
//! [ARMv7-M architecture reference manual, Appendix
//! D4](https://developer.arm.com/documentation/ddi0403/ed/). Any
//! references in this code base refers to this document.
//!
//! Aside from covering the entirety of the protocol, this crate offers
//! two iterators which reads data from the given [`Read`](std::io::Read)
//! instance:
//!
//! - [`Singles`](Singles), which decodes each packet in the stream in sequence,
//! yielding [`TracePacket`](TracePacket)s.
//!
//! - [`Timestamps`](Timestamps), which continuously decodes packets
//! from the stream until a local timestamp is encountered, yielding a
//! [`TimestampedTracePackets`](TimestampedTracePackets), which contains
//! [a timestamp relative to target reset of when the packets where
//! generated target-side](TimestampedTracePackets::timestamp).
//!
//! Usage is simple:
//! ```
//! use itm::{Decoder, DecoderOptions};
//!
//! // or a std::fs::File, or anything else that implements std::io::Read
//! let stream: &[u8] = &[
//!     // ...
//! ];
//! let mut decoder = Decoder::<&[u8]>::new(stream, DecoderOptions { ignore_eof: false });
//! for packet in decoder.singles() {
//!     // ...
//! }
//! ```
#[deny(rustdoc::broken_intra_doc_links)]
mod iter;
pub use iter::{
    LocalTimestampOptions, Singles, Timestamp, TimestampedTracePackets, Timestamps,
    TimestampsConfiguration,
};

#[cfg(feature = "serial")]
pub mod serial;

pub mod cortex_m;
use cortex_m::VectActive;

use std::convert::TryInto;
use std::io::Read;

use bitmatch::bitmatch;
use bitvec::prelude::*;

/// The set of valid packet types that can be decoded.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TracePacket {
    // Synchronization packet category (Appendix D4, p. 782)
    /// A synchronization packet is a unique pattern in the bitstream.
    /// It is identified and used to provide the alignment of other
    /// packet bytes in the bitstream. (Appendix D4.2.1)
    Sync,

    // Protocol packet category (Appendix D4, p. 782)
    /// Found in the bitstream if
    ///
    /// - Software has written to an ITM stimulus port register when the
    /// stimulus port output buffer is full.
    /// - The DWT attempts to generate a hardware source packet when the
    /// DWT output buffer is full.
    /// - The local timestamp counter overflows.
    ///
    /// See (Appendix D4.2.3).
    Overflow,

    /// A delta timestamp that measures the interval since the
    /// generation of the last local timestamp and its relation to the
    /// corresponding ITM/DWT data packets. (Appendix D4.2.4)
    LocalTimestamp1 {
        /// Timestamp value.
        ts: u32,

        /// Indicates the relationship between the generation of `ts`
        /// and the corresponding ITM or DWT data packet.
        data_relation: TimestampDataRelation,
    },

    /// A derivative of `LocalTimestamp1` for timestamp values between
    /// 1-6. Always synchronous to te associated ITM/DWT data. (Appendix D4.2.4)
    LocalTimestamp2 {
        /// Timestamp value.
        ts: u8,
    },

    /// An absolute timestamp based on the global timestamp clock that
    /// contain the timestamp's lower-order bits. (Appendix D4.2.5)
    GlobalTimestamp1 {
        /// Lower-order bits of the timestamp; bits\[25:0\].
        ts: u64,

        /// Set if higher order bits output by the last GTS2 have
        /// changed.
        wrap: bool,

        /// Set if the system has asserted a clock change input to the
        /// processor since the last generated global timestamp.
        clkch: bool,
    },

    /// An absolute timestamp based on the global timestamp clock that
    /// contain the timestamp's higher-order bits. (Appendix D4.2.5)
    GlobalTimestamp2 {
        /// Higher-order bits of the timestamp value; bits\[63:26\] or
        /// bits\[47:26\] depending on implementation.
        ts: u64,
    },

    /// A packet that provides additional information about the
    /// identified source (one of two possible, theoretically). On
    /// ARMv7-M this packet is only used to denote on which ITM stimulus
    /// port a payload was written. (Appendix D4.2.6)
    Extension {
        /// Source port page number.
        page: u8,
    },

    // Source packet category
    /// Contains the payload written to the ITM stimulus ports.
    Instrumentation {
        /// Stimulus port number.
        port: u8,

        /// Instrumentation data written to the stimulus port. MSB, BE.
        payload: Vec<u8>,
    },

    /// One or more event counters have wrapped. (Appendix D4.3.1)
    EventCounterWrap {
        /// POSTCNT wrap (see Appendix C1, p. 732).
        cyc: bool,
        /// FOLDCNT wrap (see Appendix C1, p. 734).
        fold: bool,
        /// LSUCNT wrap (see Appendix C1, p. 734).
        lsu: bool,
        /// SLEEPCNT wrap (see Appendix C1, p. 734).
        sleep: bool,
        /// EXCCNT wrap (see Appendix C1, p. 734).
        exc: bool,
        /// CPICNT wrap (see Appendix C1, p. 734).
        cpi: bool,
    },

    /// The processor has entered, exit, or returned to an exception.
    /// (Appendix D4.3.2)
    ExceptionTrace {
        exception: VectActive,
        action: ExceptionAction,
    },

    /// Periodic PC sample. (Appendix D4.3.3)
    PCSample {
        /// The value of the PC. `None` if periodic PC sleep packet.
        pc: Option<u32>,
    },

    /// A DWT comparator matched a PC value. (Appendix D4.3.4)
    DataTracePC {
        /// The comparator number that generated the data.
        comparator: u8,

        /// The PC value for the instruction that caused the successful
        /// address comparison.
        pc: u32,
    },

    /// A DWT comparator matched an address. (Appendix D4.3.4)
    DataTraceAddress {
        /// The comparator number that generated the data.
        comparator: u8,

        /// Data address content; bits\[15:0\]. MSB, BE.
        data: Vec<u8>,
    },

    /// A data trace packet with a value. (Appendix D4.3.4)
    DataTraceValue {
        /// The comparator number that generated the data.
        comparator: u8,

        /// Whether the data was read or written.
        access_type: MemoryAccessType,

        /// The data value. MSB, BE.
        value: Vec<u8>,
    },
}

/// Denotes the action taken by the processor by a given exception. (Table D4-6)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ExceptionAction {
    /// Exception was entered.
    Entered,

    /// Exception was exited.
    Exited,

    /// Exception was returned to.
    Returned,
}

/// Denotes the type of memory access.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MemoryAccessType {
    /// Memory was read.
    Read,

    /// Memory was written.
    Write,
}

/// Indicates the relationship between the generation of the local
/// timestamp packet and the corresponding ITM or DWT data packet.
/// (Appendix D4.2.4)
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TimestampDataRelation {
    /// The local timestamp value is synchronous to the corresponding
    /// ITM or DWT data. The value in the TS field is the timestamp
    /// counter value when the ITM or DWT packet is generated.
    Sync,

    /// The local timestamp value is delayed relative to the ITM or DWT
    /// data. The value in the TS field is the timestamp counter value
    /// when the Local timestamp packet is generated.
    ///
    /// Note: the local timestamp value corresponding to the previous
    /// ITM or DWT packet is unknown, but must be between the previous
    /// and the current local timestamp values.
    UnknownDelay,

    /// Output of the ITM or DWT packet corresponding to this Local
    /// timestamp packet is delayed relative to the associated event.
    /// The value in the TS field is the timestamp counter value when
    /// the ITM or DWT packets is generated.
    ///
    /// This encoding indicates that the ITM or DWT packet was delayed
    /// relative to other trace output packets.
    AssocEventDelay,

    /// Output of the ITM or DWT packet corresponding to this Local
    /// timestamp packet is delayed relative to the associated event,
    /// and this Local timestamp packet is delayed relative to the ITM
    /// or DWT data. This is a combined condition of `UnknownDelay` and
    /// `AssocEventDelay`.
    UnknownAssocEventDelay,
}

/// Set of malformed [`TracePacket`](TracePacket)s that can occur during decode.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MalformedPacket {
    /// Header is invalid and cannot be decoded.
    #[error("Header is invalid and cannot be decoded: {}", format!("{:#b}", .0))]
    InvalidHeader(u8),

    /// The type discriminator ID in the hardware source packet header
    /// is invalid or the associated payload is of wrong size.
    #[error("Hardware source packet type discriminator ID ({disc_id}) or payload length ({}) is invalid", .payload.len())]
    InvalidHardwarePacket {
        /// The discriminator ID. Potentially invalid.
        disc_id: u8,

        /// Associated payload. Potentially invalid length. MSB, BE.
        payload: Vec<u8>,
    },

    /// The type discriminator ID in the hardware source packet header
    /// is invalid.
    #[error("Hardware source packet discriminator ID is invalid: {disc_id}")]
    InvalidHardwareDisc {
        /// The discriminator ID. Potentially invalid.
        disc_id: u8,

        /// Associated payload length.
        size: usize,
    },

    /// An exception trace packet refers to an invalid action or an
    /// invalid exception number.
    #[error("IRQ number {exception} and/or action {function} is invalid")]
    InvalidExceptionTrace {
        /// The exception number.
        exception: u16,

        /// Numerical representation of the function associated with the
        /// exception number.
        function: u8,
    },

    /// The payload length of a PCSample packet is invalid.
    #[error("Payload length of PC sample is invalid: {}", .payload.len())]
    InvalidPCSampleSize {
        /// The payload constituting the PC value, of invalid size. MSB, BE.
        payload: Vec<u8>,
    },

    /// The GlobalTimestamp2 packet does not contain a 48-bit or 64-bit
    /// timestamp.
    #[error("GlobalTimestamp2 packet does not contain a 48-bit or 64-bit timestamp")]
    InvalidGTS2Size {
        /// The payload constituting the timestamp, of invalid size. MSB, BE.
        payload: Vec<u8>,
    },

    /// The number of zeroes in the Synchronization packet is less than
    /// 47.
    #[error(
        "The number of zeroes in the Synchronization packet is less than expected: {0} < {}",
        SYNC_MIN_ZEROS
    )]
    InvalidSync(usize),

    /// A source packet (from software or hardware) contains an invalid
    /// expected payload size.
    #[error(
        "A source packet (from software or hardware) contains an invalid expected payload size"
    )]
    InvalidSourcePayload {
        /// The header which contains the invalid payload size.
        header: u8,

        /// The invalid payload size. See (Appendix D4.2.8, Table D4-4).
        size: u8,
    },
}

const SYNC_MIN_ZEROS: usize = 47;

/// The decoder's possible states. The default decoder state is `Header`
/// and will always return there after a maximum of two steps. (E.g. if
/// the current state is `Syncing` or `HardwareSource`, the next state
/// is `Header` again.)
enum PacketStub {
    /// Next zero bits will be assumed to be part of a a Synchronization
    /// packet until a set bit is encountered.
    Sync(usize),

    /// Next bytes will be assumed to be part of an Instrumentation
    /// packet, until `payload` contains `expected_size` bytes.
    Instrumentation { port: u8, expected_size: usize },

    /// Next bytes will be assumed to be part of a Hardware source
    /// packet, until `payload` contains `expected_size` bytes.
    HardwareSource { disc_id: u8, expected_size: usize },

    /// Next bytes will be assumed to be part of a LocalTimestamp{1,2}
    /// packet, until the MSB is set.
    LocalTimestamp {
        data_relation: TimestampDataRelation,
    },

    /// Next bytes will be assumed to be part of a GlobalTimestamp1
    /// packet, until the MSB is set.
    GlobalTimestamp1,

    /// Next bytes will be assumed to be part of a GlobalTimestamp2
    /// packet, until the MSB is set.
    GlobalTimestamp2,
}

enum HeaderVariant {
    Packet(TracePacket),
    Stub(PacketStub),
}

/// [`Decoder`](Decoder) configuration.
pub struct DecoderOptions {
    /// Whether to keep reading after a (temporary) EOF condition. If
    /// set iteration is done over [`Singles`](Singles) or
    /// [`Timestamps`](Timestamps), [`next`](Iterator::next) will never
    /// return unless the EOF condition is eventually resolved.
    pub ignore_eof: bool,
}

#[derive(Debug, thiserror::Error)]
enum DecoderErrorInt {
    #[error("Buffer failed to read from source: {0}")]
    Io(#[from] std::io::Error),
    #[error("EOF encountered")]
    Eof,
    #[error("untars")]
    MalformedPacket(#[from] MalformedPacket),
}

/// Set of errors that can occur during decode.
#[derive(Debug, thiserror::Error)]
pub enum DecoderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("A malformed packet was encountered: {0}")]
    MalformedPacket(#[from] MalformedPacket),
}

struct Buffer<R>
where
    R: Read,
{
    reader: R,
    buffer: BitVec,
    ignore_eof: bool,
}

impl<R> Buffer<R>
where
    R: Read,
{
    pub fn new(reader: R, ignore_eof: bool) -> Buffer<R> {
        Buffer {
            reader,
            ignore_eof,
            buffer: BitVec::new(),
        }
    }

    /// Tries to read up to 32 bytes from [Self::reader]. Continuously retries if [ignore_eof] is set.
    fn buffer_some(&mut self) -> Result<(), DecoderErrorInt> {
        // `Read::read` reportedly reads in 32-byte chunks. Source:
        // <https://github.com/rust-embedded/itm/blob/3e4251b42aa2e4b05ae372c47c7b835b8acae6dc/src/lib.rs#L42>.
        let mut buffer: [u8; 32] = [0; 32];
        loop {
            match self.reader.read(&mut buffer) {
                Ok(0) => {
                    if self.ignore_eof {
                        continue;
                    }
                    return Err(DecoderErrorInt::Eof);
                }
                Ok(n) => {
                    let mut bv = BitVec::<_, LocalBits>::from_vec(buffer[0..n].to_vec());
                    bv.reverse();
                    bv.append(&mut self.buffer);
                    self.buffer.append(&mut bv);

                    return Ok(());
                }
                Err(e) => {
                    // XXX any other errors we should retry on?
                    if e.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    /// Pops a single bit from the buffer. Tries to buffer first if
    /// the buffer is empty.
    pub fn pop_bit(&mut self) -> Result<bool, DecoderErrorInt> {
        loop {
            match self.buffer.pop() {
                None => {
                    self.buffer_some()?;
                    continue;
                }
                Some(bit) => return Ok(bit),
            }
        }
    }

    /// Pops a single byte from the buffer. Tries to buffer if more data
    /// is needed.
    pub fn pop_byte(&mut self) -> Result<u8, DecoderErrorInt> {
        let mut b: u8 = 0;
        for i in 0..8 {
            b |= (self.pop_bit()? as u8) << i;
        }

        Ok(b)
    }

    /// Pops `cnt` bytes from the buffer. Tries to buffer if more data
    /// is needed.
    pub fn pop_bytes(&mut self, cnt: usize) -> Result<Vec<u8>, DecoderErrorInt> {
        let mut bytes = vec![];
        for _ in 0..cnt {
            bytes.push(self.pop_byte()?);
        }

        Ok(bytes)
    }

    /// Pops bytes from the incoming buffer until the continuation-bit
    /// is not set. All [TracePacket]s with a defined payload follow
    /// this payload schema. (c.f. e.g. Appendix D4, Fig. D4-4)
    #[bitmatch]
    pub fn pop_payload(&mut self) -> Result<Vec<u8>, DecoderErrorInt> {
        let mut payload = vec![];
        loop {
            let b = self.pop_byte()?;
            payload.push(b);

            #[bitmatch]
            let "c???_????" = b;
            if c == 0 {
                break;
            }
        }

        Ok(payload)
    }
}

/// ITM/DWT packet protocol decoder.
pub struct Decoder<R>
where
    R: Read,
{
    /// Intermediate buffer to store the trace byte stream read from the
    /// given [Read] instance.
    buffer: Buffer<R>,

    /// Whether the decoder is in a state of synchronization.
    sync: Option<usize>,
}

impl<R> Decoder<R>
where
    R: Read,
{
    pub fn new(reader: R, options: DecoderOptions) -> Decoder<R> {
        Decoder {
            buffer: Buffer::new(reader, options.ignore_eof),
            sync: None,
        }
    }

    /// Returns a reference to the underlying [`Read`](Read).
    pub fn get_ref(&self) -> &R {
        &self.buffer.reader
    }

    /// Returns a mutable reference to the underlying [`Read`](Read).
    pub fn get_mut(&mut self) -> &mut R {
        &mut self.buffer.reader
    }

    /// Returns an iterator over [`TracePacket`](TracePacket)s. Consumes
    /// the [`Decoder`](Decoder).
    pub fn singles(self) -> Singles<R> {
        Singles::new(self)
    }

    /// Returns an iterator over
    /// [`TimestampedTracePackets`](TimestampedTracePackets). Consumes
    /// the [`Decoder`](Decoder).
    ///
    /// # Panics
    ///
    /// This iterator constuctor will panic if
    /// [`options.lts_prescaler`](TimestampsConfiguration::lts_prescaler)
    /// is [`Disabled`](LocalTimestampOptions::Disabled).
    pub fn timestamps(self, options: TimestampsConfiguration) -> Timestamps<R> {
        Timestamps::new(self, options)
    }

    /// Returns the next [TracePacket] in the stream.
    fn next_single(&mut self) -> Result<TracePacket, DecoderErrorInt> {
        if self.sync.is_some() {
            return self.handle_sync();
        }
        assert!(self.sync.is_none());

        match decode_header(self.buffer.pop_byte()?)? {
            HeaderVariant::Packet(p) => Ok(p),
            HeaderVariant::Stub(s) => self.process_stub(&s),
        }
    }

    /// Read zeros from the bitstream until the first bit is set. This
    /// realigns the incoming bitstream for further processing, which
    /// broke alignment on target-generated overflow packet.
    fn handle_sync(&mut self) -> Result<TracePacket, DecoderErrorInt> {
        let zeros = self.sync.unwrap();
        match (self.buffer.pop_bit()?, zeros) {
            (true, zeros) if zeros >= SYNC_MIN_ZEROS => {
                self.sync = None;
                Ok(TracePacket::Sync)
            }
            (true, zeros) if zeros < SYNC_MIN_ZEROS => {
                self.sync = None;
                Err(MalformedPacket::InvalidSync(zeros).into())
            }
            (false, _) => {
                *self.sync.as_mut().unwrap() += 1;
                self.handle_sync()
            }
            (true, _) => unreachable!(),
        }
    }

    #[bitmatch]
    fn process_stub(&mut self, stub: &PacketStub) -> Result<TracePacket, DecoderErrorInt> {
        match stub {
            PacketStub::Sync(count) => {
                self.sync = Some(*count);
                self.handle_sync()
            }

            PacketStub::HardwareSource {
                disc_id,
                expected_size,
            } => {
                let payload = self.buffer.pop_bytes(*expected_size)?;
                handle_hardware_source(*disc_id, payload).map_err(DecoderErrorInt::MalformedPacket)
            }
            PacketStub::LocalTimestamp { data_relation } => {
                let payload = self.buffer.pop_payload()?;
                Ok(TracePacket::LocalTimestamp1 {
                    data_relation: data_relation.clone(),
                    // MAGIC(27): c.f. Appendix D4.2.4
                    ts: extract_timestamp(payload, 27) as u32,
                })
            }
            PacketStub::GlobalTimestamp1 => {
                let payload = self.buffer.pop_payload()?;
                #[bitmatch]
                let "?wc?_????" = payload.last().unwrap();

                Ok(TracePacket::GlobalTimestamp1 {
                    clkch: c > 0,
                    wrap: w > 0,
                    // MAGIC(25): c.f. Appendix D4.2.5
                    ts: extract_timestamp(payload, 25),
                })
            }
            PacketStub::GlobalTimestamp2 => {
                let payload = self.buffer.pop_payload()?;
                Ok(TracePacket::GlobalTimestamp2 {
                    ts: extract_timestamp(
                        payload.to_vec(),
                        match payload.len() {
                            4 => 47 - 26, // 48 bit timestamp
                            6 => 63 - 26, // 64 bit timestamp
                            _ => {
                                return Err(DecoderErrorInt::MalformedPacket(
                                    MalformedPacket::InvalidGTS2Size {
                                        payload: payload.to_vec(),
                                    },
                                ))
                            }
                        },
                    ),
                })
            }
            PacketStub::Instrumentation {
                port,
                expected_size,
            } => {
                let payload = self.buffer.pop_bytes(*expected_size)?;
                Ok(TracePacket::Instrumentation {
                    port: *port,
                    payload,
                })
            }
        }
    }
}

// TODO template this for u32, u64?
fn extract_timestamp(payload: Vec<u8>, max_len: u64) -> u64 {
    // Decode the first N - 1 payload bytes
    let (rtail, head) = payload.split_at(payload.len() - 1);
    let mut ts: u64 = 0;
    for (i, b) in rtail.iter().enumerate() {
        ts |= ((b & !(1 << 7)) as u64) // mask out continuation bit
            << (7 * i);
    }

    // Mask out the timestamp's MSBs and shift them into the final
    // value.
    let shift = 7 - (max_len % 7);
    let mask: u8 = 0xFFu8.wrapping_shl(shift.try_into().unwrap()) >> shift;
    ts | (((head[0] & mask) as u64) << (7 * rtail.len()))
}

/// Decodes the first byte of a packet, the header, into a complete packet or a packet stub.
#[allow(clippy::bad_bit_mask)]
#[bitmatch]
fn decode_header(header: u8) -> Result<HeaderVariant, MalformedPacket> {
    fn translate_ss(ss: u8) -> Option<usize> {
        // See (Appendix D4.2.8, Table D4-4)
        Some(
            match ss {
                0b01 => 2,
                0b10 => 3,
                0b11 => 5,
                _ => return None,
            } - 1, // ss would include the header byte, but it has already been processed
        )
    }

    let stub = |s| Ok(HeaderVariant::Stub(s));
    let packet = |p| Ok(HeaderVariant::Packet(p));

    #[bitmatch]
    match header {
        // Synchronization packet category
        "0000_0000" => stub(PacketStub::Sync(8)),

        // Protocol packet category
        "0111_0000" => packet(TracePacket::Overflow),
        "11rr_0000" => {
            // Local timestamp, format 1 (LTS1)
            let tc = r; // relationship with corresponding data

            stub(PacketStub::LocalTimestamp {
                data_relation: match tc {
                    0b00 => TimestampDataRelation::Sync,
                    0b01 => TimestampDataRelation::UnknownDelay,
                    0b10 => TimestampDataRelation::AssocEventDelay,
                    0b11 => TimestampDataRelation::UnknownAssocEventDelay,
                    _ => unreachable!(),
                },
            })
        }
        "0ttt_0000" => {
            // Local timestamp, format 2 (LTS2)
            packet(TracePacket::LocalTimestamp2 { ts: t })
        }
        "1001_0100" => {
            // Global timestamp, format 1 (GTS1)
            stub(PacketStub::GlobalTimestamp1)
        }
        "1011_0100" => {
            // Global timestamp, format 2(GTS2)
            stub(PacketStub::GlobalTimestamp2)
        }
        "0ppp_1000" => {
            // Extension packet
            packet(TracePacket::Extension { page: p })
        }

        // Source packet category
        "aaaa_a0ss" => {
            // Instrumentation packet
            stub(PacketStub::Instrumentation {
                port: a,
                expected_size: if let Some(s) = translate_ss(s) {
                    s
                } else {
                    return Err(MalformedPacket::InvalidSourcePayload { header, size: s });
                },
            })
        }
        "aaaa_a1ss" => {
            // Hardware source packet
            let disc_id = a;

            if !(0..=2).contains(&disc_id) && !(8..=23).contains(&disc_id) {
                return Err(MalformedPacket::InvalidHardwareDisc {
                    disc_id,
                    size: s.into(),
                });
            }

            stub(PacketStub::HardwareSource {
                disc_id,
                expected_size: if let Some(s) = translate_ss(s) {
                    s
                } else {
                    return Err(MalformedPacket::InvalidSourcePayload { header, size: s });
                },
            })
        }
        #[allow(clippy::identity_op)]
        "hhhh_hhhh" => Err(MalformedPacket::InvalidHeader(h)),
    }
}

/// Decodes the payload of a hardware source packet.
#[bitmatch]
fn handle_hardware_source(disc_id: u8, payload: Vec<u8>) -> Result<TracePacket, MalformedPacket> {
    match disc_id {
        0 => {
            // event counter wrap

            if payload.len() != 1 {
                return Err(MalformedPacket::InvalidHardwarePacket { disc_id, payload });
            }

            #[bitmatch]
            let "??yf_lsec" = payload[0];
            Ok(TracePacket::EventCounterWrap {
                cyc: y != 0,
                fold: f != 0,
                lsu: l != 0,
                sleep: s != 0,
                exc: e != 0,
                cpi: c != 0,
            })
        }
        1 => {
            // exception trace

            if payload.len() != 2 {
                return Err(MalformedPacket::InvalidHardwarePacket { disc_id, payload });
            }

            #[bitmatch]
            let "??ff_???e" = payload[1];
            let exception_number = ((e as u16) << 8) | payload[0] as u16;

            Ok(TracePacket::ExceptionTrace {
                exception: if let Some(exception) = VectActive::from(exception_number) {
                    exception
                } else {
                    return Err(MalformedPacket::InvalidExceptionTrace {
                        exception: exception_number,
                        function: f,
                    });
                },
                action: match f {
                    0b01 => ExceptionAction::Entered,
                    0b10 => ExceptionAction::Exited,
                    0b11 => ExceptionAction::Returned,
                    _ => {
                        return Err(MalformedPacket::InvalidExceptionTrace {
                            exception: exception_number,
                            function: f,
                        })
                    }
                },
            })
        }
        2 => {
            // PC sample
            match payload.len() {
                1 if payload[0] == 0 => Ok(TracePacket::PCSample { pc: None }),
                4 => Ok(TracePacket::PCSample {
                    pc: Some(u32::from_le_bytes(payload.try_into().unwrap())),
                }),
                _ => Err(MalformedPacket::InvalidPCSampleSize { payload }),
            }
        }
        8..=23 => {
            // data trace
            #[bitmatch]
            let "???t_tccd" = disc_id; // we have already masked out bit[2:0]
            let comparator = c;

            match (t, d, payload.len()) {
                (0b01, 0, 4) => {
                    // PC value packet
                    Ok(TracePacket::DataTracePC {
                        comparator,
                        pc: u32::from_le_bytes(payload.try_into().unwrap()),
                    })
                }
                (0b01, 1, 2) => {
                    // address packet
                    Ok(TracePacket::DataTraceAddress {
                        comparator,
                        data: payload,
                    })
                }
                (0b10, d, _) => {
                    // data value packet
                    Ok(TracePacket::DataTraceValue {
                        comparator,
                        access_type: if d == 0 {
                            MemoryAccessType::Read
                        } else {
                            MemoryAccessType::Write
                        },
                        value: payload,
                    })
                }
                _ => Err(MalformedPacket::InvalidHardwarePacket { disc_id, payload }),
            }
        }
        _ => unreachable!(), // we already verify the discriminator when we decode the header
    }
}

#[cfg(test)]
mod decoder_buffer_utils {
    use super::*;

    #[test]
    fn buffer_pop_bytes() {
        let bytes: &[u8] = &[0b1000_0000, 0b1010_0000, 0b1000_0100, 0b0110_0000];
        let mut decoder = Decoder::new(bytes, DecoderOptions { ignore_eof: false });

        assert_eq!(decoder.buffer.pop_bytes(3).unwrap().len(), 3);
    }

    #[test]
    fn buffer_pop_payload() {
        #[rustfmt::skip]
        let payload: &[u8] = &[
            0b1000_0000,
            0b1010_0000,
            0b1000_0100,
            0b0110_0000
        ];
        let mut decoder = Decoder::new(payload, DecoderOptions { ignore_eof: false });

        assert_eq!(decoder.buffer.pop_payload().unwrap(), payload);
    }

    #[test]
    fn extract_timestamp() {
        #[rustfmt::skip]
        let ts: Vec<u8> = [
            0b1000_0000,
            0b1000_0000,
            0b1000_0000,
            0b0000_0000,
        ].to_vec();

        assert_eq!(super::extract_timestamp(ts, 25), 0);

        #[rustfmt::skip]
        let ts: Vec<u8> = [
            0b1000_0001,
            0b1000_0111,
            0b1001_1111,
            0b0111_1111
        ].to_vec();

        assert_eq!(
            super::extract_timestamp(ts, 27),
            0b1111111_0011111_0000111_0000001,
        );

        #[rustfmt::skip]
        let ts: Vec<u8> = [
            0b1000_0001,
            0b1000_0111,
            0b1001_1111,
            0b1111_1111
        ].to_vec();

        assert_eq!(
            super::extract_timestamp(ts, 25),
            0b11111_0011111_0000111_0000001,
        );
    }
}
