use anyhow::{bail, Context, Result};
use itm::{Decoder, DecoderOptions, LocalTimestampOptions, TimestampsConfiguration};
use std::fs::File;
use std::path::PathBuf;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(
    about = "An ITM/DWT packet protocol decoder, as specified in the ARMv7-M architecture reference manual, Appendix D4. See <https://developer.arm.com/documentation/ddi0403/ed/>. Report bugs and request features at <https://github.com/rust-embedded/itm>."
)]
struct Opt {
    #[structopt(long = "--ignore-eof")]
    ignore_eof: bool,

    #[structopt(long = "--timestamps", requires("freq"))]
    timestamps: bool,

    #[structopt(long = "--itm-prescaler")]
    prescaler: Option<u8>,

    #[structopt(long = "--itm-freq", name = "freq")]
    freq: Option<u32>,

    #[structopt(long = "--expect-malformed")]
    expect_malformed: bool,

    #[structopt(name = "FILE", parse(from_os_str), help = "Raw trace input file.")]
    file: PathBuf,
}

fn main() -> Result<()> {
    let opt = Opt::from_args();

    let file = File::open(&opt.file).context("failed to open file")?;
    let decoder = Decoder::<File>::new(
        file,
        DecoderOptions {
            ignore_eof: opt.ignore_eof,
        },
    );

    match opt {
        Opt {
            timestamps: true,
            prescaler,
            freq: Some(freq),
            expect_malformed,
            ..
        } => {
            for packets in decoder.timestamps(TimestampsConfiguration {
                clock_frequency: freq,
                lts_prescaler: match prescaler {
                    None | Some(1) => LocalTimestampOptions::Enabled,
                    Some(4) => LocalTimestampOptions::EnabledDiv4,
                    Some(16) => LocalTimestampOptions::EnabledDiv16,
                    Some(64) => LocalTimestampOptions::EnabledDiv64,
                    Some(n) => bail!(
                        "{} is not a valid prescaler; valid prescalers are: 4, 16, 64.",
                        n
                    ),
                },
                expect_malformed: expect_malformed,
            }) {
                match packets {
                    Err(e) => return Err(e).context("Decoder error"),
                    Ok(packets) => println!("{:?}", packets),
                }
            }
        }
        _ => {
            for packet in decoder.singles() {
                match packet {
                    Err(e) => return Err(e).context("Decoder error"),
                    Ok(packet) => println!("{:?}", packet),
                }
            }
        }
    }

    Ok(())
}
