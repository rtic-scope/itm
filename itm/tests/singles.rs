use itm::*;

#[test]
fn eof() {
    let empty: &[u8] = &[];
    let decoder = Decoder::new(empty, DecoderOptions { ignore_eof: false });

    assert!(decoder.singles().next().is_none());
}

#[test]
fn decode_sync_packet() {
    let mut trace_data: Vec<u8> = [0; 47 / 8].to_vec();
    trace_data.push(1 << 7);

    let decoder = Decoder::new(trace_data.as_slice(), DecoderOptions { ignore_eof: false });
    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::Sync
    );
}

#[test]
fn decode_overflow_packet() {
    let overflow: &[u8] = &[0b0111_0000];
    let decoder = Decoder::new(overflow, DecoderOptions { ignore_eof: false });
    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::Overflow
    );
}

#[test]
fn decode_local_timestamp_packets() {
    let lts: &[u8] = &[
        // LTS1
        0b1100_0000,
        0b1100_1001,
        0b0000_0001,
        // LTS2
        0b0101_0000,
    ];
    let mut decoder = Decoder::new(lts, DecoderOptions { ignore_eof: false }).singles();

    for packet in [
        TracePacket::LocalTimestamp1 {
            ts: 0b11001001,
            data_relation: TimestampDataRelation::Sync,
        },
        TracePacket::LocalTimestamp2 { ts: 0b101 },
    ]
    .iter()
    .cloned()
    {
        assert_eq!(decoder.next().unwrap().unwrap(), packet);
    }
}

#[test]
fn decode_global_timestamp_packets() {
    let gts: &[u8] = &[
        // GTS1
        0b1001_0100,
        0b1000_0000,
        0b1010_0000,
        0b1000_0100,
        0b0110_0000,
        // GTS2 (48-bit)
        0b1011_0100,
        0b1011_1101,
        0b1111_0100,
        0b1001_0001,
        0b0000_0001,
        // GTS2 (64-bit)
        0b1011_0100,
        0b1011_1101,
        0b1111_0100,
        0b1001_0001,
        0b1000_0001,
        0b1111_0100,
        0b0000_0111,
    ];
    let mut decoder = Decoder::new(gts, DecoderOptions { ignore_eof: false }).singles();

    for packet in [
        TracePacket::GlobalTimestamp1 {
            ts: 0b00000_0000100_0100000_0000000,
            wrap: true,
            clkch: true,
        },
        TracePacket::GlobalTimestamp2 {
            ts: 0b1_0010001_1110100_0111101,
        },
        TracePacket::GlobalTimestamp2 {
            ts: 0b111_1110100_0000001_0010001_1110100_0111101,
        },
    ]
    .iter()
    .cloned()
    {
        assert_eq!(decoder.next().unwrap().unwrap(), packet);
    }
}

#[test]
fn decode_extention_packet() {
    let ext: &[u8] = &[0b0111_1000];
    let decoder = Decoder::new(ext, DecoderOptions { ignore_eof: false });
    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::Extension { page: 0b111 }
    );
}

#[test]
fn decode_instrumentation_packet() {
    let instr: &[u8] = &[
        0b1000_1011,
        0b0000_0011,
        0b0000_1111,
        0b0011_1111,
        0b1111_1111,
    ];
    let decoder = Decoder::new(instr, DecoderOptions { ignore_eof: false });

    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::Instrumentation {
            port: 0b1000_1,
            #[rustfmt::skip]
                payload: [
                    0b0000_0011,
                    0b0000_1111,
                    0b0011_1111,
                    0b1111_1111,
                ].to_vec(),
        }
    );
}

#[test]
fn decode_eventcounterwrap_packet() {
    #[rustfmt::skip]
        let event: &[u8] = &[
            0b0000_0101,
            0b0010_1010
        ];
    let decoder = Decoder::new(event, DecoderOptions { ignore_eof: false });

    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::EventCounterWrap {
            cyc: true,
            fold: false,
            lsu: true,
            sleep: false,
            exc: true,
            cpi: false,
        }
    );
}

#[test]
fn decode_exceptiontrace_packet() {
    #[rustfmt::skip]
        let excpt: &[u8] = &[
            0b0000_1110,
            0b0010_0000,
            0b0011_0000
        ];
    let decoder = Decoder::new(excpt, DecoderOptions { ignore_eof: false });

    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::ExceptionTrace {
            exception: cortex_m::peripheral::scb::VectActive::Interrupt { irqn: 16 },
            action: ExceptionAction::Returned,
        }
    );
}

#[test]
fn decode_pcsample_packet() {
    let samples: &[u8] = &[
        // PC sample (not sleeping)
        0b0001_0111,
        0b0000_0011,
        0b0000_1111,
        0b0011_1111,
        0b1111_1111,
        // PC sample (sleeping)
        0b0001_0101,
        0b0000_0000,
    ];
    let mut decoder = Decoder::new(samples, DecoderOptions { ignore_eof: false }).singles();

    for packet in [
        TracePacket::PCSample {
            pc: Some(0b11111111_00111111_00001111_00000011),
        },
        TracePacket::PCSample { pc: None },
    ]
    .iter()
    .cloned()
    {
        assert_eq!(decoder.next().unwrap().unwrap(), packet);
    }
}

#[test]
fn decode_datatracepc_packet() {
    let pc: &[u8] = &[
        0b0111_0111,
        0b0000_0011,
        0b0000_1111,
        0b0011_1111,
        0b1111_1111,
    ];
    let decoder = Decoder::new(pc, DecoderOptions { ignore_eof: false });

    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::DataTracePC {
            comparator: 0b11,
            pc: 0b11111111_00111111_00001111_00000011,
        }
    );
}

#[test]
fn decode_datatraceaddress_packet() {
    #[rustfmt::skip]
        let address: &[u8] = &[
            0b0110_1110,
            0b0000_0011,
            0b0000_1111,
        ];
    let decoder = Decoder::new(address, DecoderOptions { ignore_eof: false });

    assert_eq!(
        decoder.singles().next().unwrap().unwrap(),
        TracePacket::DataTraceAddress {
            comparator: 0b10,
            #[rustfmt::skip]
                data: [
                    0b0000_0011,
                    0b0000_1111,
                ].to_vec(),
        }
    );
}

#[test]
fn decode_datatracevalue_packet() {
    let payloads: &[u8] = &[
        // four-byte (word) payload
        0b1010_1111,
        0b0000_0011,
        0b0000_1111,
        0b0011_1111,
        0b1111_1111,
        // two-byte (halfword) payload
        0b1010_1110,
        0b0000_0011,
        0b0000_1111,
        // one-byte (byte) payload
        0b1010_1101,
        0b0000_0011,
    ];
    let mut decoder = Decoder::new(payloads, DecoderOptions { ignore_eof: false }).singles();

    for packet in [
        TracePacket::DataTraceValue {
            comparator: 0b10,
            access_type: MemoryAccessType::Write,
            #[rustfmt::skip]
                value: [
                    0b0000_0011,
                    0b0000_1111,
                    0b0011_1111,
                    0b1111_1111,
                ].to_vec(),
        },
        TracePacket::DataTraceValue {
            comparator: 0b10,
            access_type: MemoryAccessType::Write,
            #[rustfmt::skip]
                value: [
                    0b0000_0011,
                    0b0000_1111,
                ].to_vec(),
        },
        TracePacket::DataTraceValue {
            comparator: 0b10,
            access_type: MemoryAccessType::Write,
            #[rustfmt::skip]
                value: [
                    0b0000_0011,
                ].to_vec(),
        },
    ]
    .iter()
    .cloned()
    {
        assert_eq!(decoder.next().unwrap().unwrap(), packet);
    }
}
