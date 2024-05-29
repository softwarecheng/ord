#![allow(
    clippy::large_enum_variant,
    clippy::result_large_err,
    clippy::too_many_arguments,
    clippy::type_complexity
)]
#![deny(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

use {
    self::{
        arguments::Arguments,
        blocktime::Blocktime,
        decimal::Decimal,
        inscriptions::{
            inscription_id,
            media::{self, ImageRendering, Media},
            teleburn, Charm, ParsedEnvelope,
        },
        representation::Representation,
        runes::{Pile, SpacedRune},
        settings::Settings,
        subcommand::{Subcommand, SubcommandResult},
        tally::Tally,
    },
    anyhow::{anyhow, bail, ensure, Context, Error},
    bitcoincore_rpc::{Client, RpcApi},
    bitcoint4::{
        address::{Address, NetworkUnchecked},
        blockdata::constants::{DIFFCHANGE_INTERVAL, SUBSIDY_HALVING_INTERVAL},
        consensus::{self, Decodable, Encodable},
        hash_types::{BlockHash, TxMerkleNode},
        hashes::Hash,
        opcodes,
        script::{self, Instruction},
        Amount, Block, Network, OutPoint, Script, ScriptBuf, Sequence, Transaction, TxIn, TxOut,
        Txid,
    },
    chrono::{DateTime, TimeZone, Utc},
    ciborium::Value,
    clap::{ArgGroup, Parser},
    html_escaper::{Escape, Trusted},
    lazy_static::lazy_static,
    ordinals::{DeserializeFromStr, Epoch, Height, Rarity, Sat, SatPoint},
    regex::Regex,
    reqwest::Url,
    serde::{Deserialize, Deserializer, Serialize, Serializer},
    std::{
        cmp::{self, Reverse},
        collections::{BTreeMap, HashMap, HashSet, VecDeque},
        env,
        fmt::{self, Display, Formatter},
        fs::{self, File},
        io::{self, Cursor, Read},
        net::ToSocketAddrs,
        path::{Path, PathBuf},
        process::{self, Command, Stdio},
        str::FromStr,
        sync::{
            atomic::{self, AtomicBool},
            Arc, Mutex,
        },
        thread,
        time::{Duration, Instant, SystemTime},
    },
    sysinfo::System,
    tokio::{runtime::Runtime, task},
};

pub use self::{
    chain::Chain,
    fee_rate::FeeRate,
    index::{Index, MintEntry, RuneEntry},
    inscriptions::{Envelope, Inscription, InscriptionId},
    object::Object,
    options::Options,
    runes::{Edict, Rune, RuneId, Runestone},
};

pub mod api;
pub mod arguments;
mod blocktime;
pub mod chain;
mod decimal;
mod fee_rate;
pub mod index;
mod inscriptions;
mod object;
pub mod options;
pub mod outgoing;
mod representation;
pub mod runes;
mod server_config;
mod settings;
pub mod subcommand;
mod tally;
pub mod templates;

type Result<T = (), E = Error> = std::result::Result<T, E>;

static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
static LISTENERS: Mutex<Vec<axum_server::Handle>> = Mutex::new(Vec::new());
static INDEXER: Mutex<Option<thread::JoinHandle<()>>> = Mutex::new(None);

pub fn timestamp(seconds: u32) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds.into(), 0).unwrap()
}

fn target_as_block_hash(target: bitcoint4::Target) -> BlockHash {
    BlockHash::from_raw_hash(Hash::from_byte_array(target.to_le_bytes()))
}

fn unbound_outpoint() -> OutPoint {
    OutPoint {
        txid: Hash::all_zeros(),
        vout: 0,
    }
}

pub fn parse_ord_server_args(args: &str) -> (Settings, subcommand::server::Server) {
    match Arguments::try_parse_from(args.split_whitespace()) {
        Ok(arguments) => match arguments.subcommand {
            Subcommand::Server(server) => (
                Settings::merge(
                    arguments.options,
                    vec![("INTEGRATION_TEST".into(), "1".into())]
                        .into_iter()
                        .collect(),
                )
                .unwrap(),
                server,
            ),
            subcommand => panic!("unexpected subcommand: {subcommand:?}"),
        },
        Err(err) => panic!("error parsing arguments: {err}"),
    }
}

fn gracefully_shutdown_indexer() {
    if let Some(indexer) = INDEXER.lock().unwrap().take() {
        // We explicitly set this to true to notify the thread to not take on new work
        SHUTTING_DOWN.store(true, atomic::Ordering::Relaxed);
        log::info!("Waiting for index thread to finish...");
        if indexer.join().is_err() {
            log::warn!("Index thread panicked; join failed");
        }
    }
}

pub fn main() {
    env_logger::init();

    ctrlc::set_handler(move || {
        if SHUTTING_DOWN.fetch_or(true, atomic::Ordering::Relaxed) {
            process::exit(1);
        }

        println!("Shutting down gracefully. Press <CTRL-C> again to shutdown immediately.");

        LISTENERS
            .lock()
            .unwrap()
            .iter()
            .for_each(|handle| handle.graceful_shutdown(Some(Duration::from_millis(100))));

        gracefully_shutdown_indexer();
    })
    .expect("Error setting <CTRL-C> handler");

    let args = Arguments::parse();

    let minify = args.options.minify;

    match args.run() {
        Err(err) => {
            eprintln!("error: {err}");
            err.chain()
                .skip(1)
                .for_each(|cause| eprintln!("because: {cause}"));
            if env::var_os("RUST_BACKTRACE")
                .map(|val| val == "1")
                .unwrap_or_default()
            {
                eprintln!("{}", err.backtrace());
            }

            gracefully_shutdown_indexer();

            process::exit(1);
        }
        Ok(output) => {
            if let Some(output) = output {
                output.print_json(minify);
            }
            gracefully_shutdown_indexer();
        }
    }
}
