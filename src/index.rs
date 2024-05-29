use {
    self::{
        entry::{
            Entry, HeaderValue, InscriptionEntry, InscriptionEntryValue, InscriptionIdValue,
            OutPointValue, RuneEntryValue, RuneIdValue, SatPointValue, SatRange, TxidValue,
        },
        event::Event,
        reorg::*,
        runes::{Rune, RuneId},
        updater::Updater,
    },
    super::*,
    crate::{
        subcommand::{find::FindRangeOutput, server::query},
        templates::StatusHtml,
    },
    bitcoincore_rpc::{
        json::{GetBlockHeaderResult, GetBlockStatsResult},
        Client,
    },
    bitcoint4::block::Header,
    chrono::SubsecRound,
    indicatif::{ProgressBar, ProgressStyle},
    log::log_enabled,
    rayon::prelude::*,
    redb::{
        Database, DatabaseError, MultimapTable, MultimapTableDefinition, MultimapTableHandle,
        ReadOnlyTable, ReadableMultimapTable, ReadableTable, RepairSession, StorageError, Table,
        TableDefinition, TableHandle, TableStats, WriteTransaction,
    },
    std::{
        collections::HashMap,
        fs::OpenOptions,
        io::{BufReader, BufWriter, Seek, SeekFrom, Write},
        sync::{Mutex, Once},
    },
};

pub use {self::entry::RuneEntry, entry::MintEntry};

pub(crate) mod entry;
pub mod event;
mod fetcher;
mod reorg;
mod rtx;
mod updater;

const SCHEMA_VERSION: u64 = 18;

macro_rules! define_table {
    ($name:ident, $key:ty, $value:ty) => {
        const $name: TableDefinition<$key, $value> = TableDefinition::new(stringify!($name));
    };
}

macro_rules! define_multimap_table {
    ($name:ident, $key:ty, $value:ty) => {
        const $name: MultimapTableDefinition<$key, $value> =
            MultimapTableDefinition::new(stringify!($name));
    };
}

define_multimap_table! { SATPOINT_TO_SEQUENCE_NUMBER, &SatPointValue, u32 }
define_multimap_table! { SAT_TO_SEQUENCE_NUMBER, u64, u32 }
define_multimap_table! { SEQUENCE_NUMBER_TO_CHILDREN, u32, u32 }
define_table! { CONTENT_TYPE_TO_COUNT, Option<&[u8]>, u64 }
define_table! { HEIGHT_TO_BLOCK_HEADER, u32, &HeaderValue }
define_table! { HEIGHT_TO_LAST_SEQUENCE_NUMBER, u32, u32 }
define_table! { HOME_INSCRIPTIONS, u32, InscriptionIdValue }
define_table! { INSCRIPTION_ID_TO_SEQUENCE_NUMBER, InscriptionIdValue, u32 }
define_table! { INSCRIPTION_NUMBER_TO_SEQUENCE_NUMBER, i32, u32 }
define_table! { OUTPOINT_TO_RUNE_BALANCES, &OutPointValue, &[u8] }
define_table! { OUTPOINT_TO_SAT_RANGES, &OutPointValue, &[u8] }
define_table! { OUTPOINT_TO_VALUE, &OutPointValue, u64}
define_table! { RUNE_ID_TO_RUNE_ENTRY, RuneIdValue, RuneEntryValue }
define_table! { RUNE_TO_RUNE_ID, u128, RuneIdValue }
define_table! { SAT_TO_SATPOINT, u64, &SatPointValue }
define_table! { SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY, u32, InscriptionEntryValue }
define_table! { SEQUENCE_NUMBER_TO_RUNE_ID, u32, RuneIdValue }
define_table! { SEQUENCE_NUMBER_TO_SATPOINT, u32, &SatPointValue }
define_table! { STATISTIC_TO_COUNT, u64, u64 }
define_table! { TRANSACTION_ID_TO_RUNE, &TxidValue, u128 }
define_table! { TRANSACTION_ID_TO_TRANSACTION, &TxidValue, &[u8] }
define_table! { WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP, u32, u128 }

#[derive(Copy, Clone)]
pub(crate) enum Statistic {
    Schema = 0,
    BlessedInscriptions = 1,
    Commits = 2,
    CursedInscriptions = 3,
    IndexRunes = 4,
    IndexSats = 5,
    LostSats = 6,
    OutputsTraversed = 7,
    ReservedRunes = 8,
    Runes = 9,
    SatRanges = 10,
    UnboundInscriptions = 11,
    IndexTransactions = 12,
    IndexSpentSats = 13,
    InitialSyncTime = 14,
}

impl Statistic {
    fn key(self) -> u64 {
        self.into()
    }
}

impl From<Statistic> for u64 {
    fn from(statistic: Statistic) -> Self {
        statistic as u64
    }
}

#[derive(Serialize)]
pub(crate) struct Info {
    blocks_indexed: u32,
    branch_pages: u64,
    fragmented_bytes: u64,
    index_file_size: u64,
    index_path: PathBuf,
    leaf_pages: u64,
    metadata_bytes: u64,
    outputs_traversed: u64,
    page_size: usize,
    sat_ranges: u64,
    stored_bytes: u64,
    tables: BTreeMap<String, TableInfo>,
    total_bytes: u64,
    pub(crate) transactions: Vec<TransactionInfo>,
    tree_height: u32,
    utxos_indexed: u64,
}

#[derive(Serialize)]
pub(crate) struct TableInfo {
    branch_pages: u64,
    fragmented_bytes: u64,
    leaf_pages: u64,
    metadata_bytes: u64,
    proportion: f64,
    stored_bytes: u64,
    total_bytes: u64,
    tree_height: u32,
}

impl From<TableStats> for TableInfo {
    fn from(stats: TableStats) -> Self {
        Self {
            branch_pages: stats.branch_pages(),
            fragmented_bytes: stats.fragmented_bytes(),
            leaf_pages: stats.leaf_pages(),
            metadata_bytes: stats.metadata_bytes(),
            proportion: 0.0,
            stored_bytes: stats.stored_bytes(),
            total_bytes: stats.stored_bytes() + stats.metadata_bytes() + stats.fragmented_bytes(),
            tree_height: stats.tree_height(),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct TransactionInfo {
    pub(crate) starting_block_count: u32,
    pub(crate) starting_timestamp: u128,
}

pub(crate) struct InscriptionInfo {
    pub(crate) children: Vec<InscriptionId>,
    pub(crate) entry: InscriptionEntry,
    pub(crate) parent: Option<InscriptionId>,
    pub(crate) output: Option<TxOut>,
    pub(crate) satpoint: SatPoint,
    pub(crate) inscription: Inscription,
    pub(crate) previous: Option<InscriptionId>,
    pub(crate) next: Option<InscriptionId>,
    pub(crate) rune: Option<SpacedRune>,
    pub(crate) charms: u16,
}

pub(crate) trait BitcoinCoreRpcResultExt<T> {
    fn into_option(self) -> Result<Option<T>>;
}

impl<T> BitcoinCoreRpcResultExt<T> for Result<T, bitcoincore_rpc::Error> {
    fn into_option(self) -> Result<Option<T>> {
        match self {
            Ok(ok) => Ok(Some(ok)),
            Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
                bitcoincore_rpc::jsonrpc::error::RpcError { code: -8, .. },
            ))) => Ok(None),
            Err(bitcoincore_rpc::Error::JsonRpc(bitcoincore_rpc::jsonrpc::error::Error::Rpc(
                bitcoincore_rpc::jsonrpc::error::RpcError { message, .. },
            ))) if message.ends_with("not found") => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

pub struct Index {
    client: Client,
    database: Database,
    durability: redb::Durability,
    event_sender: Option<tokio::sync::mpsc::Sender<Event>>,
    first_inscription_height: u32,
    genesis_block_coinbase_transaction: Transaction,
    genesis_block_coinbase_txid: Txid,
    height_limit: Option<u32>,
    index_runes: bool,
    index_sats: bool,
    index_spent_sats: bool,
    index_transactions: bool,
    settings: Settings,
    path: PathBuf,
    started: DateTime<Utc>,
    unrecoverably_reorged: AtomicBool,
}

impl Index {
    pub fn open(settings: &Settings) -> Result<Self> {
        Index::open_with_event_sender(settings, None)
    }

    pub fn open_with_event_sender(
        settings: &Settings,
        event_sender: Option<tokio::sync::mpsc::Sender<Event>>,
    ) -> Result<Self> {
        let client = settings.bitcoin_rpc_client(None)?;

        let path = settings.index().to_owned();

        if let Err(err) = fs::create_dir_all(path.parent().unwrap()) {
            bail!(
                "failed to create data dir `{}`: {err}",
                path.parent().unwrap().display()
            );
        }

        let index_cache_size = settings.index_cache_size();

        log::info!("Setting index cache size to {} bytes", index_cache_size);

        let durability = if cfg!(test) {
            redb::Durability::None
        } else {
            redb::Durability::Immediate
        };

        let index_path = path.clone();
        let once = Once::new();
        let progress_bar = Mutex::new(None);

        let repair_callback = move |progress: &mut RepairSession| {
            once.call_once(|| println!("Index file `{}` needs recovery. This can take a long time, especially for the --index-sats index.", index_path.display()));

            if !(cfg!(test) || log_enabled!(log::Level::Info)) {
                let mut guard = progress_bar.lock().unwrap();

                let progress_bar = guard.get_or_insert_with(|| {
                    let progress_bar = ProgressBar::new(100);
                    progress_bar.set_style(
                        ProgressStyle::with_template("[repairing database] {wide_bar} {pos}/{len}")
                            .unwrap(),
                    );
                    progress_bar
                });

                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                progress_bar.set_position((progress.progress() * 100.0) as u64);
            }
        };

        let database = match Database::builder()
            .set_cache_size(index_cache_size)
            .set_repair_callback(repair_callback)
            .open(&path)
        {
            Ok(database) => {
                {
                    let schema_version = database
                        .begin_read()?
                        .open_table(STATISTIC_TO_COUNT)?
                        .get(&Statistic::Schema.key())?
                        .map(|x| x.value())
                        .unwrap_or(0);

                    match schema_version.cmp(&SCHEMA_VERSION) {
            cmp::Ordering::Less =>
              bail!(
                "index at `{}` appears to have been built with an older, incompatible version of ord, consider deleting and rebuilding the index: index schema {schema_version}, ord schema {SCHEMA_VERSION}",
                path.display()
              ),
            cmp::Ordering::Greater =>
              bail!(
                "index at `{}` appears to have been built with a newer, incompatible version of ord, consider updating ord: index schema {schema_version}, ord schema {SCHEMA_VERSION}",
                path.display()
              ),
            cmp::Ordering::Equal => {
            }
          }
                }

                database
            }
            Err(DatabaseError::Storage(StorageError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                let database = Database::builder()
                    .set_cache_size(index_cache_size)
                    .create(&path)?;

                let mut tx = database.begin_write()?;

                tx.set_durability(durability);

                tx.open_multimap_table(SATPOINT_TO_SEQUENCE_NUMBER)?;
                tx.open_multimap_table(SAT_TO_SEQUENCE_NUMBER)?;
                tx.open_multimap_table(SEQUENCE_NUMBER_TO_CHILDREN)?;
                tx.open_table(CONTENT_TYPE_TO_COUNT)?;
                tx.open_table(HEIGHT_TO_BLOCK_HEADER)?;
                tx.open_table(HEIGHT_TO_LAST_SEQUENCE_NUMBER)?;
                tx.open_table(HOME_INSCRIPTIONS)?;
                tx.open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?;
                tx.open_table(INSCRIPTION_NUMBER_TO_SEQUENCE_NUMBER)?;
                tx.open_table(OUTPOINT_TO_RUNE_BALANCES)?;
                tx.open_table(OUTPOINT_TO_VALUE)?;
                tx.open_table(RUNE_ID_TO_RUNE_ENTRY)?;
                tx.open_table(RUNE_TO_RUNE_ID)?;
                tx.open_table(SAT_TO_SATPOINT)?;
                tx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;
                tx.open_table(SEQUENCE_NUMBER_TO_RUNE_ID)?;
                tx.open_table(SEQUENCE_NUMBER_TO_SATPOINT)?;
                tx.open_table(TRANSACTION_ID_TO_RUNE)?;
                tx.open_table(WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP)?;

                {
                    let mut outpoint_to_sat_ranges = tx.open_table(OUTPOINT_TO_SAT_RANGES)?;
                    let mut statistics = tx.open_table(STATISTIC_TO_COUNT)?;

                    if settings.index_sats() {
                        outpoint_to_sat_ranges.insert(&OutPoint::null().store(), [].as_slice())?;
                    }

                    Self::set_statistic(
                        &mut statistics,
                        Statistic::IndexRunes,
                        u64::from(settings.index_runes()),
                    )?;

                    Self::set_statistic(
                        &mut statistics,
                        Statistic::IndexSats,
                        u64::from(settings.index_sats() || settings.index_spent_sats()),
                    )?;

                    Self::set_statistic(
                        &mut statistics,
                        Statistic::IndexSpentSats,
                        u64::from(settings.index_spent_sats()),
                    )?;

                    Self::set_statistic(
                        &mut statistics,
                        Statistic::IndexTransactions,
                        u64::from(settings.index_transactions()),
                    )?;

                    Self::set_statistic(&mut statistics, Statistic::Schema, SCHEMA_VERSION)?;
                }

                tx.commit()?;

                database
            }
            Err(error) => bail!("failed to open index: {error}"),
        };

        let index_runes;
        let index_sats;
        let index_spent_sats;
        let index_transactions;

        {
            let tx = database.begin_read()?;
            let statistics = tx.open_table(STATISTIC_TO_COUNT)?;
            index_runes = Self::is_statistic_set(&statistics, Statistic::IndexRunes)?;
            index_sats = Self::is_statistic_set(&statistics, Statistic::IndexSats)?;
            index_spent_sats = Self::is_statistic_set(&statistics, Statistic::IndexSpentSats)?;
            index_transactions = Self::is_statistic_set(&statistics, Statistic::IndexTransactions)?;
        }

        let genesis_block_coinbase_transaction =
            settings.chain().genesis_block().coinbase().unwrap().clone();

        Ok(Self {
            genesis_block_coinbase_txid: genesis_block_coinbase_transaction.txid(),
            client,
            database,
            durability,
            event_sender,
            first_inscription_height: settings.first_inscription_height(),
            genesis_block_coinbase_transaction,
            height_limit: settings.height_limit(),
            index_runes,
            index_sats,
            index_spent_sats,
            index_transactions,
            settings: settings.clone(),
            path,
            started: Utc::now(),
            unrecoverably_reorged: AtomicBool::new(false),
        })
    }

    pub(crate) fn contains_output(&self, output: &OutPoint) -> Result<bool> {
        Ok(self
            .database
            .begin_read()?
            .open_table(OUTPOINT_TO_VALUE)?
            .get(&output.store())?
            .is_some())
    }

    pub(crate) fn has_rune_index(&self) -> bool {
        self.index_runes
    }

    pub(crate) fn has_sat_index(&self) -> bool {
        self.index_sats
    }

    pub(crate) fn status(&self) -> Result<StatusHtml> {
        let rtx = self.database.begin_read()?;

        let statistic_to_count = rtx.open_table(STATISTIC_TO_COUNT)?;

        let statistic = |statistic: Statistic| -> Result<u64> {
            Ok(statistic_to_count
                .get(statistic.key())?
                .map(|guard| guard.value())
                .unwrap_or_default())
        };

        let height = rtx
            .open_table(HEIGHT_TO_BLOCK_HEADER)?
            .range(0..)?
            .next_back()
            .transpose()?
            .map(|(height, _header)| height.value());

        let next_height = height.map(|height| height + 1).unwrap_or(0);

        let blessed_inscriptions = statistic(Statistic::BlessedInscriptions)?;
        let cursed_inscriptions = statistic(Statistic::CursedInscriptions)?;
        let initial_sync_time = statistic(Statistic::InitialSyncTime)?;

        let mut content_type_counts = rtx
            .open_table(CONTENT_TYPE_TO_COUNT)?
            .iter()?
            .map(|result| {
                result.map(|(key, value)| (key.value().map(|slice| slice.into()), value.value()))
            })
            .collect::<Result<Vec<(Option<Vec<u8>>, u64)>, StorageError>>()?;

        content_type_counts.sort_by_key(|(_content_type, count)| Reverse(*count));

        Ok(StatusHtml {
            blessed_inscriptions,
            chain: self.settings.chain(),
            content_type_counts,
            cursed_inscriptions,
            height,
            initial_sync_time: Duration::from_micros(initial_sync_time),
            inscriptions: blessed_inscriptions + cursed_inscriptions,
            lost_sats: statistic(Statistic::LostSats)?,
            minimum_rune_for_next_block: Rune::minimum_at_height(
                self.settings.chain(),
                Height(next_height),
            ),
            rune_index: statistic(Statistic::IndexRunes)? != 0,
            runes: statistic(Statistic::Runes)?,
            sat_index: statistic(Statistic::IndexSats)? != 0,
            started: self.started,
            transaction_index: statistic(Statistic::IndexTransactions)? != 0,
            unrecoverably_reorged: self.unrecoverably_reorged.load(atomic::Ordering::Relaxed),
            uptime: (Utc::now() - self.started).to_std()?,
        })
    }

    pub(crate) fn info(&self) -> Result<Info> {
        let stats = self.database.begin_write()?.stats()?;

        let rtx = self.database.begin_read()?;

        let mut tables: BTreeMap<String, TableInfo> = BTreeMap::new();

        for handle in rtx.list_tables()? {
            let name = handle.name().into();
            let stats = rtx.open_untyped_table(handle)?.stats()?;
            tables.insert(name, stats.into());
        }

        for handle in rtx.list_multimap_tables()? {
            let name = handle.name().into();
            let stats = rtx.open_untyped_multimap_table(handle)?.stats()?;
            tables.insert(name, stats.into());
        }

        for table in rtx.list_tables()? {
            assert!(tables.contains_key(table.name()));
        }

        for table in rtx.list_multimap_tables()? {
            assert!(tables.contains_key(table.name()));
        }

        let total_bytes = tables
            .values()
            .map(|table_info| table_info.total_bytes)
            .sum();

        tables.values_mut().for_each(|table_info| {
            table_info.proportion = table_info.total_bytes as f64 / total_bytes as f64
        });

        let info = {
            let statistic_to_count = rtx.open_table(STATISTIC_TO_COUNT)?;
            let sat_ranges = statistic_to_count
                .get(&Statistic::SatRanges.key())?
                .map(|x| x.value())
                .unwrap_or(0);
            let outputs_traversed = statistic_to_count
                .get(&Statistic::OutputsTraversed.key())?
                .map(|x| x.value())
                .unwrap_or(0);
            Info {
                index_path: self.path.clone(),
                blocks_indexed: rtx
                    .open_table(HEIGHT_TO_BLOCK_HEADER)?
                    .range(0..)?
                    .next_back()
                    .transpose()?
                    .map(|(height, _header)| height.value() + 1)
                    .unwrap_or(0),
                branch_pages: stats.branch_pages(),
                fragmented_bytes: stats.fragmented_bytes(),
                index_file_size: fs::metadata(&self.path)?.len(),
                leaf_pages: stats.leaf_pages(),
                metadata_bytes: stats.metadata_bytes(),
                sat_ranges,
                outputs_traversed,
                page_size: stats.page_size(),
                stored_bytes: stats.stored_bytes(),
                total_bytes,
                tables,
                transactions: rtx
                    .open_table(WRITE_TRANSACTION_STARTING_BLOCK_COUNT_TO_TIMESTAMP)?
                    .range(0..)?
                    .flat_map(|result| {
                        result.map(
                            |(starting_block_count, starting_timestamp)| TransactionInfo {
                                starting_block_count: starting_block_count.value(),
                                starting_timestamp: starting_timestamp.value(),
                            },
                        )
                    })
                    .collect(),
                tree_height: stats.tree_height(),
                utxos_indexed: rtx.open_table(OUTPOINT_TO_SAT_RANGES)?.len()?,
            }
        };

        Ok(info)
    }

    pub fn update(&self) -> Result {
        loop {
            let wtx = self.begin_write()?;

            let mut updater = Updater {
                height: wtx
                    .open_table(HEIGHT_TO_BLOCK_HEADER)?
                    .range(0..)?
                    .next_back()
                    .transpose()?
                    .map(|(height, _header)| height.value() + 1)
                    .unwrap_or(0),
                index: self,
                outputs_cached: 0,
                outputs_inserted_since_flush: 0,
                outputs_traversed: 0,
                range_cache: HashMap::new(),
                sat_ranges_since_flush: 0,
            };

            match updater.update_index(wtx) {
                Ok(ok) => return Ok(ok),
                Err(err) => {
                    log::info!("{}", err.to_string());

                    match err.downcast_ref() {
                        Some(&ReorgError::Recoverable { height, depth }) => {
                            Reorg::handle_reorg(self, height, depth)?;
                        }
                        Some(&ReorgError::Unrecoverable) => {
                            self.unrecoverably_reorged
                                .store(true, atomic::Ordering::Relaxed);
                            return Err(anyhow!(ReorgError::Unrecoverable));
                        }
                        _ => return Err(err),
                    };
                }
            }
        }
    }

    pub(crate) fn export(&self, filename: &String, include_addresses: bool) -> Result {
        let mut writer = BufWriter::new(File::create(filename)?);
        let rtx = self.database.begin_read()?;

        let blocks_indexed = rtx
            .open_table(HEIGHT_TO_BLOCK_HEADER)?
            .range(0..)?
            .next_back()
            .transpose()?
            .map(|(height, _header)| height.value() + 1)
            .unwrap_or(0);

        writeln!(writer, "# export at block height {}", blocks_indexed)?;

        log::info!("exporting database tables to {filename}");

        let sequence_number_to_satpoint = rtx.open_table(SEQUENCE_NUMBER_TO_SATPOINT)?;

        for result in rtx
            .open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?
            .iter()?
        {
            let entry = result?;
            let sequence_number = entry.0.value();
            let entry = InscriptionEntry::load(entry.1.value());
            let satpoint = SatPoint::load(
                *sequence_number_to_satpoint
                    .get(sequence_number)?
                    .unwrap()
                    .value(),
            );

            write!(
                writer,
                "{}\t{}\t{}",
                entry.inscription_number, entry.id, satpoint
            )?;

            if include_addresses {
                let address = if satpoint.outpoint == unbound_outpoint() {
                    "unbound".to_string()
                } else {
                    let output = self
                        .get_transaction(satpoint.outpoint.txid)?
                        .unwrap()
                        .output
                        .into_iter()
                        .nth(satpoint.outpoint.vout.try_into().unwrap())
                        .unwrap();
                    self.settings
                        .chain()
                        .address_from_script(&output.script_pubkey)
                        .map(|address| address.to_string())
                        .unwrap_or_else(|e| e.to_string())
                };
                write!(writer, "\t{}", address)?;
            }
            writeln!(writer)?;

            if SHUTTING_DOWN.load(atomic::Ordering::Relaxed) {
                break;
            }
        }
        writer.flush()?;
        Ok(())
    }

    pub(crate) fn get_ordx_block_inscription(
        &self,
        chain: Chain,
        first_block_txid: &str,
        inscription_id: &InscriptionId,
    ) -> Result<api::OrdxBlockInscription, Error> {
        let query_inscription_id = query::Inscription::Id(*inscription_id);
        // println!("export block-> height: {height} , inscriptionid: {query_inscription_id} , firstBlockTxid: {first_block_txid}");
        let info = Index::inscription_info(self, query_inscription_id)?.ok_or_else(|| {
            anyhow::Error::msg(format!("inscription {query_inscription_id} not found"))
        })?;

        let api_inscription = api::OrdxInscription {
            address: info
                .output
                .as_ref()
                .and_then(|o| chain.address_from_script(&o.script_pubkey).ok())
                .map(|address| address.to_string()),
            // charms: Charm::ALL
            //   .iter()
            //   .filter(|charm| charm.is_set(info.charms))
            //   .map(|charm| charm.title().into())
            //   .collect(),
            children: info.children,
            content_length: info.inscription.content_length(),
            content_type: info.inscription.content_type().map(|s| s.to_string()),
            fee: info.entry.fee,
            height: info.entry.height,
            id: info.entry.id,
            next: info.next,
            number: info.entry.inscription_number,
            parent: info.parent,
            previous: info.previous,
            // rune: info.rune,
            sat: info.entry.sat,
            satpoint: info.satpoint,
            timestamp: timestamp(info.entry.timestamp).timestamp(),
            value: info.output.as_ref().map(|o| o.value),
        };

        // api output
        // let unbound_output = unbound_outpoint();
        // let ordx_output = match api_inscription.satpoint.outpoint != unbound_output {
        //   true => {
        //     let outpoint = api_inscription.satpoint.outpoint;
        //     // let sat_ranges = self.list(outpoint)?;
        //     // let inscriptions = self.get_inscriptions_on_output(outpoint)?;
        //     // let indexed = self.contains_output(&outpoint)?;
        //     // let runes = self.get_rune_balances_for_outpoint(outpoint)?;
        //     // let spent = self.is_output_spent(outpoint)?;
        //     let output = self
        //       .get_transaction(outpoint.txid)?
        //       .ok_or_else(|| anyhow::Error::msg(format!("output {outpoint}")))?
        //       .output
        //       .into_iter()
        //       .nth(outpoint.vout as usize)
        //       .ok_or_else(|| anyhow::Error::msg(format!("output {outpoint}")))?;
        //     Some(api::OrdxOutput::new(chain, outpoint, output))
        //   }
        //   false => None,
        // };

        // get geneses address from address
        // When the output and inciption id are different, it means that the inscription has been traded, else this is first block tx
        let mut outpoint = api_inscription.satpoint.outpoint;
        if api_inscription.satpoint.outpoint.txid != inscription_id.txid
            && api_inscription.satpoint.outpoint.txid.to_string() != first_block_txid
        {
            let mut output_index = inscription_id.index;
            let transaction = self.get_transaction(inscription_id.txid)?.ok_or_else(|| {
                anyhow::Error::msg(format!("transaction {}", inscription_id.txid))
            })?;
            let output_len = transaction.output.len() as u32;
            // cursed and blessed inscription share the same outpoint, ex: tx 219a5e5458bf0ba686f1c5660cf01652c88dec1b30c13571c43d97a9b11ac653
            while output_index >= output_len {
                output_index -= 1;
            }
            outpoint = OutPoint::new(inscription_id.txid, output_index)
        }
        // let sat_ranges = self.list(outpoint)?;
        // let inscriptions = self.get_inscriptions_on_output(outpoint)?;
        // let indexed = self.contains_output(&outpoint)?;
        // let runes = self.get_rune_balances_for_outpoint(outpoint)?;
        // let spent = self.is_output_spent(outpoint)?;
        let output = self
            .get_transaction(outpoint.txid)?
            .ok_or_else(|| anyhow::Error::msg(format!("output {outpoint}")))?
            .output
            .into_iter()
            .nth(outpoint.vout as usize)
            .ok_or_else(|| anyhow::Error::msg(format!("output {outpoint}")))?;
        // let cloned_output = output.clone();
        // let api_geneses_output = api::Output::new(
        //   chain,
        //   inscriptions,
        //   outpoint,
        //   output,
        //   indexed,
        //   runes,
        //   sat_ranges,
        //   spent,
        // );
        let geneses_address = chain
            .address_from_script(&output.script_pubkey)
            .map_or_else(
                |_| {
                    println!(
                        "no find address-> outpoint: {}, output.script_pubkey: {}",
                        outpoint, output.script_pubkey
                    );
                    "".to_string()
                },
                |address| address.to_string(),
            );
        // .ok()
        // .map(|address| address.to_string())
        // .ok_or_else(|| {
        //   anyhow::Error::msg(format!("output.script_pubkey: {}", output.script_pubkey))
        // })?;

        Ok(api::OrdxBlockInscription {
            genesesaddress: geneses_address,
            inscription: api_inscription,
            // output: ordx_output.unwrap_or_default(),
        })
    }

    pub(crate) fn export_ordx(
        &self,
        filename: &String,
        cache: u64,
        parallel: bool,
        chain: Chain,
        mut first_inscription_height: u32,
    ) -> Result {
        let start_time = Instant::now();
        let path = Path::new(filename);
        if let Some(parent_dir) = path.parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir)?;
            }
        } else {
            return Err(anyhow!(
                "ordx data directory is not a valid path: {}",
                path.parent().unwrap().display()
            ));
        }

        let rtx = self.database.begin_read()?;
        let blocks_indexed = rtx
            .open_table(HEIGHT_TO_BLOCK_HEADER)?
            .range(0..)?
            .next_back()
            .transpose()?
            .map(|(height, _header)| height.value() + 1)
            .unwrap_or(0);

        let mut file_size = 0;
        if Path::new(filename).exists() {
            let file = OpenOptions::new().read(true).open(filename)?;
            let metadata = fs::metadata(filename)?;
            file_size = metadata.len();

            let mut reader = BufReader::new(file);

            let last_line = if file_size > 0 {
                reader.seek(SeekFrom::End(-1))?;
                let mut pos = reader.stream_position()?;
                let mut last_char = [0; 1];
                reader.read_exact(&mut last_char)?;

                if last_char == [b'\n'] {
                    pos -= 1;
                }

                let mut buffer = Vec::new();
                while pos > 0 {
                    reader.seek(SeekFrom::Start(pos))?;
                    if reader.read(&mut last_char)? == 0 || last_char == [b'\n'] {
                        break;
                    }
                    buffer.push(last_char[0]);
                    pos -= 1;
                }
                if pos == 0 {
                    reader.seek(SeekFrom::Start(0))?;
                }
                buffer.reverse();
                String::from_utf8_lossy(&buffer).trim_end().to_string()
            } else {
                String::new()
            };

            if !last_line.is_empty() {
                let ordx_block_inscriptions: api::OrdxBlockInscriptions =
                    serde_json::from_str(&last_line)?;
                first_inscription_height = ordx_block_inscriptions.height + 1;
            }
        }
        let max_block_num = blocks_indexed - 1;
        println!(
            "block {first_inscription_height}->{max_block_num}, export {filename}, size: {:.2}MB.",
            file_size as f64 / (1024.0 * 1024.0)
        );

        let mut writer = BufWriter::new(
            OpenOptions::new()
                .write(true)
                .append(true)
                .create(true)
                .open(filename)?,
        );
        let mut need_flush = false;
        let mut flush_block_number: u64 = 0;
        let mut flush_inscription_number: u64 = 0;
        let mut total_inscription_number: u64 = 0;
        for height in first_inscription_height..=max_block_num {
            let block = self
                .get_block_by_height(height)?
                .ok_or_else(|| anyhow::Error::msg(format!("block {height}")))?;
            let inscription_id_list = self.get_inscriptions_in_block(height)?;
            let first_block_txid = match block.txdata.len() > 0 {
                true => block.txdata[0].txid().to_string(),
                false => Txid::all_zeros().to_string(),
            };

            let inscriptions = if parallel {
                inscription_id_list
                    .par_iter()
                    .map(|inscription_id| {
                        self.get_ordx_block_inscription(
                            chain.clone(),
                            &first_block_txid,
                            inscription_id,
                        )
                    })
                    .collect::<Result<Vec<api::OrdxBlockInscription>, Error>>()?
            } else {
                inscription_id_list
                    .iter()
                    .map(|inscription_id| {
                        self.get_ordx_block_inscription(
                            chain.clone(),
                            &first_block_txid,
                            inscription_id,
                        )
                    })
                    .collect::<Result<Vec<api::OrdxBlockInscription>, Error>>()?
            };

            let ordx_block_inscriptions = api::OrdxBlockInscriptions {
                height,
                inscriptions,
            };

            if !need_flush {
                need_flush = ordx_block_inscriptions.inscriptions.len() > 0;
            }

            if height == first_inscription_height {
                write!(writer, "\n")?;
            }

            if ordx_block_inscriptions.inscriptions.len() > 0 {
                let json = serde_json::to_string(&ordx_block_inscriptions)?;
                write!(writer, "{}\n", json)?;
                flush_block_number += 1;
                flush_inscription_number += ordx_block_inscriptions.inscriptions.len() as u64;

                println!(
                    "export block-> height: {height}, inscription count: {}",
                    ordx_block_inscriptions.inscriptions.len()
                );
            }

            if need_flush && (flush_inscription_number % cache == 0 || height == max_block_num) {
                writer.flush()?;
                need_flush = false;
                total_inscription_number += flush_inscription_number;
                flush_inscription_number = 0;
                println!("export block-> already flush block number: {flush_block_number}, flush inscription count: {total_inscription_number}");
            }
            if SHUTTING_DOWN.load(atomic::Ordering::Relaxed) {
                break;
            }
        }

        writer.flush()?;
        let duration = start_time.elapsed();
        let mut block_number = 0;
        if max_block_num >= first_inscription_height {
            block_number = max_block_num - first_inscription_height + 1;
        }
        println!(
            "complete! scan block number {block_number}, write block number {flush_block_number}, \
total elapsed {}s.",
            duration.as_secs()
        );
        Ok(())
    }

    fn begin_read(&self) -> Result<rtx::Rtx> {
        Ok(rtx::Rtx(self.database.begin_read()?))
    }

    fn begin_write(&self) -> Result<WriteTransaction> {
        let mut tx = self.database.begin_write()?;
        tx.set_durability(self.durability);
        Ok(tx)
    }

    fn increment_statistic(wtx: &WriteTransaction, statistic: Statistic, n: u64) -> Result {
        let mut statistic_to_count = wtx.open_table(STATISTIC_TO_COUNT)?;
        let value = statistic_to_count
            .get(&(statistic.key()))?
            .map(|x| x.value())
            .unwrap_or_default()
            + n;
        statistic_to_count.insert(&statistic.key(), &value)?;
        Ok(())
    }

    pub(crate) fn set_statistic(
        statistics: &mut Table<u64, u64>,
        statistic: Statistic,
        value: u64,
    ) -> Result<()> {
        statistics.insert(&statistic.key(), &value)?;
        Ok(())
    }

    pub(crate) fn is_statistic_set(
        statistics: &ReadOnlyTable<u64, u64>,
        statistic: Statistic,
    ) -> Result<bool> {
        Ok(statistics
            .get(&statistic.key())?
            .map(|guard| guard.value())
            .unwrap_or_default()
            != 0)
    }

    pub(crate) fn block_count(&self) -> Result<u32> {
        self.begin_read()?.block_count()
    }

    pub(crate) fn block_height(&self) -> Result<Option<Height>> {
        self.begin_read()?.block_height()
    }

    pub(crate) fn block_hash(&self, height: Option<u32>) -> Result<Option<BlockHash>> {
        self.begin_read()?.block_hash(height)
    }

    pub(crate) fn blocks(&self, take: usize) -> Result<Vec<(u32, BlockHash)>> {
        let rtx = self.begin_read()?;

        let block_count = rtx.block_count()?;

        let height_to_block_header = rtx.0.open_table(HEIGHT_TO_BLOCK_HEADER)?;

        let mut blocks = Vec::with_capacity(block_count.try_into().unwrap());

        for next in height_to_block_header
            .range(0..block_count)?
            .rev()
            .take(take)
        {
            let next = next?;
            blocks.push((next.0.value(), Header::load(*next.1.value()).block_hash()));
        }

        Ok(blocks)
    }

    pub(crate) fn rare_sat_satpoints(&self) -> Result<Vec<(Sat, SatPoint)>> {
        let rtx = self.database.begin_read()?;

        let sat_to_satpoint = rtx.open_table(SAT_TO_SATPOINT)?;

        let mut result = Vec::with_capacity(sat_to_satpoint.len()?.try_into().unwrap());

        for range in sat_to_satpoint.range(0..)? {
            let (sat, satpoint) = range?;
            result.push((Sat(sat.value()), Entry::load(*satpoint.value())));
        }

        Ok(result)
    }

    pub(crate) fn rare_sat_satpoint(&self, sat: Sat) -> Result<Option<SatPoint>> {
        Ok(self
            .database
            .begin_read()?
            .open_table(SAT_TO_SATPOINT)?
            .get(&sat.n())?
            .map(|satpoint| Entry::load(*satpoint.value())))
    }

    pub(crate) fn get_rune_by_id(&self, id: RuneId) -> Result<Option<Rune>> {
        Ok(self
            .database
            .begin_read()?
            .open_table(RUNE_ID_TO_RUNE_ENTRY)?
            .get(&id.store())?
            .map(|entry| RuneEntry::load(entry.value()).rune))
    }

    pub(crate) fn rune(
        &self,
        rune: Rune,
    ) -> Result<Option<(RuneId, RuneEntry, Option<InscriptionId>)>> {
        let rtx = self.database.begin_read()?;

        let Some(id) = rtx
      .open_table(RUNE_TO_RUNE_ID)?
      .get(rune.0)?
      .map(|guard| guard.value())
    else {
      return Ok(None);
    };

        let entry = RuneEntry::load(
            rtx.open_table(RUNE_ID_TO_RUNE_ENTRY)?
                .get(id)?
                .unwrap()
                .value(),
        );

        let parent = InscriptionId {
            txid: entry.etching,
            index: 0,
        };

        let parent = rtx
            .open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?
            .get(&parent.store())?
            .is_some()
            .then_some(parent);

        Ok(Some((RuneId::load(id), entry, parent)))
    }

    pub(crate) fn runes(&self) -> Result<Vec<(RuneId, RuneEntry)>> {
        let mut entries = Vec::new();

        for result in self
            .database
            .begin_read()?
            .open_table(RUNE_ID_TO_RUNE_ENTRY)?
            .iter()?
        {
            let (id, entry) = result?;
            entries.push((RuneId::load(id.value()), RuneEntry::load(entry.value())));
        }

        Ok(entries)
    }

    pub(crate) fn get_rune_balances_for_outpoint(
        &self,
        outpoint: OutPoint,
    ) -> Result<Vec<(SpacedRune, Pile)>> {
        let rtx = self.database.begin_read()?;

        let outpoint_to_balances = rtx.open_table(OUTPOINT_TO_RUNE_BALANCES)?;

        let id_to_rune_entries = rtx.open_table(RUNE_ID_TO_RUNE_ENTRY)?;

        let Some(balances) = outpoint_to_balances.get(&outpoint.store())? else {
      return Ok(Vec::new());
    };

        let balances_buffer = balances.value();

        let mut balances = Vec::new();
        let mut i = 0;
        while i < balances_buffer.len() {
            let (id, length) = runes::varint::decode(&balances_buffer[i..]);
            i += length;
            let (amount, length) = runes::varint::decode(&balances_buffer[i..]);
            i += length;

            let id = RuneId::try_from(id).unwrap();

            let entry = RuneEntry::load(id_to_rune_entries.get(id.store())?.unwrap().value());

            balances.push((
                entry.spaced_rune(),
                Pile {
                    amount,
                    divisibility: entry.divisibility,
                    symbol: entry.symbol,
                },
            ));
        }

        Ok(balances)
    }

    pub(crate) fn get_rune_balance_map(&self) -> Result<BTreeMap<Rune, BTreeMap<OutPoint, u128>>> {
        let outpoint_balances = self.get_rune_balances()?;

        let rtx = self.database.begin_read()?;

        let rune_id_to_rune_entry = rtx.open_table(RUNE_ID_TO_RUNE_ENTRY)?;

        let mut rune_balances: BTreeMap<Rune, BTreeMap<OutPoint, u128>> = BTreeMap::new();

        for (outpoint, balances) in outpoint_balances {
            for (rune_id, amount) in balances {
                let rune = RuneEntry::load(
                    rune_id_to_rune_entry
                        .get(&rune_id.store())?
                        .unwrap()
                        .value(),
                )
                .rune;

                *rune_balances
                    .entry(rune)
                    .or_default()
                    .entry(outpoint)
                    .or_default() += amount;
            }
        }

        Ok(rune_balances)
    }

    pub(crate) fn get_rune_balances(&self) -> Result<Vec<(OutPoint, Vec<(RuneId, u128)>)>> {
        let mut result = Vec::new();

        for entry in self
            .database
            .begin_read()?
            .open_table(OUTPOINT_TO_RUNE_BALANCES)?
            .iter()?
        {
            let (outpoint, balances_buffer) = entry?;
            let outpoint = OutPoint::load(*outpoint.value());
            let balances_buffer = balances_buffer.value();

            let mut balances = Vec::new();
            let mut i = 0;
            while i < balances_buffer.len() {
                let (id, length) = runes::varint::decode(&balances_buffer[i..]);
                i += length;
                let (balance, length) = runes::varint::decode(&balances_buffer[i..]);
                i += length;
                balances.push((RuneId::try_from(id)?, balance));
            }

            result.push((outpoint, balances));
        }

        Ok(result)
    }

    pub(crate) fn block_header(&self, hash: BlockHash) -> Result<Option<Header>> {
        self.client.get_block_header(&hash).into_option()
    }

    pub(crate) fn block_header_info(
        &self,
        hash: BlockHash,
    ) -> Result<Option<GetBlockHeaderResult>> {
        self.client.get_block_header_info(&hash).into_option()
    }

    pub(crate) fn block_stats(&self, height: u64) -> Result<Option<GetBlockStatsResult>> {
        self.client.get_block_stats(height).into_option()
    }

    pub(crate) fn get_block_by_height(&self, height: u32) -> Result<Option<Block>> {
        Ok(self
            .client
            .get_block_hash(height.into())
            .into_option()?
            .map(|hash| self.client.get_block(&hash))
            .transpose()?)
    }

    pub(crate) fn get_block_by_hash(&self, hash: BlockHash) -> Result<Option<Block>> {
        self.client.get_block(&hash).into_option()
    }

    pub(crate) fn get_collections_paginated(
        &self,
        page_size: usize,
        page_index: usize,
    ) -> Result<(Vec<InscriptionId>, bool)> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let mut collections = rtx
            .open_multimap_table(SEQUENCE_NUMBER_TO_CHILDREN)?
            .iter()?
            .skip(page_index.saturating_mul(page_size))
            .take(page_size.saturating_add(1))
            .map(|result| {
                result
                    .and_then(|(parent, _children)| {
                        sequence_number_to_inscription_entry
                            .get(parent.value())
                            .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                    })
                    .map_err(|err| err.into())
            })
            .collect::<Result<Vec<InscriptionId>>>()?;

        let more = collections.len() > page_size;

        if more {
            collections.pop();
        }

        Ok((collections, more))
    }

    pub(crate) fn get_children_by_sequence_number_paginated(
        &self,
        sequence_number: u32,
        page_size: usize,
        page_index: usize,
    ) -> Result<(Vec<InscriptionId>, bool)> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_entry = rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let mut children = rtx
            .open_multimap_table(SEQUENCE_NUMBER_TO_CHILDREN)?
            .get(sequence_number)?
            .skip(page_index * page_size)
            .take(page_size.saturating_add(1))
            .map(|result| {
                result
                    .and_then(|sequence_number| {
                        sequence_number_to_entry
                            .get(sequence_number.value())
                            .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                    })
                    .map_err(|err| err.into())
            })
            .collect::<Result<Vec<InscriptionId>>>()?;

        let more = children.len() > page_size;

        if more {
            children.pop();
        }

        Ok((children, more))
    }

    pub(crate) fn get_etching(&self, txid: Txid) -> Result<Option<SpacedRune>> {
        let rtx = self.database.begin_read()?;

        let transaction_id_to_rune = rtx.open_table(TRANSACTION_ID_TO_RUNE)?;
        let Some(rune) = transaction_id_to_rune.get(&txid.store())? else {
      return Ok(None);
    };

        let rune_to_rune_id = rtx.open_table(RUNE_TO_RUNE_ID)?;
        let id = rune_to_rune_id.get(rune.value())?.unwrap();

        let rune_id_to_rune_entry = rtx.open_table(RUNE_ID_TO_RUNE_ENTRY)?;
        let entry = rune_id_to_rune_entry.get(&id.value())?.unwrap();

        Ok(Some(RuneEntry::load(entry.value()).spaced_rune()))
    }

    pub(crate) fn get_inscription_ids_by_sat(&self, sat: Sat) -> Result<Vec<InscriptionId>> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let ids = rtx
            .open_multimap_table(SAT_TO_SEQUENCE_NUMBER)?
            .get(&sat.n())?
            .map(|result| {
                result
                    .and_then(|sequence_number| {
                        let sequence_number = sequence_number.value();
                        sequence_number_to_inscription_entry
                            .get(sequence_number)
                            .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                    })
                    .map_err(|err| err.into())
            })
            .collect::<Result<Vec<InscriptionId>>>()?;

        Ok(ids)
    }

    pub(crate) fn get_inscription_ids_by_sat_paginated(
        &self,
        sat: Sat,
        page_size: u64,
        page_index: u64,
    ) -> Result<(Vec<InscriptionId>, bool)> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let mut ids = rtx
            .open_multimap_table(SAT_TO_SEQUENCE_NUMBER)?
            .get(&sat.n())?
            .skip(page_index.saturating_mul(page_size).try_into().unwrap())
            .take(page_size.saturating_add(1).try_into().unwrap())
            .map(|result| {
                result
                    .and_then(|sequence_number| {
                        let sequence_number = sequence_number.value();
                        sequence_number_to_inscription_entry
                            .get(sequence_number)
                            .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                    })
                    .map_err(|err| err.into())
            })
            .collect::<Result<Vec<InscriptionId>>>()?;

        let more = ids.len() > page_size.try_into().unwrap();

        if more {
            ids.pop();
        }

        Ok((ids, more))
    }

    pub(crate) fn get_inscription_id_by_sat_indexed(
        &self,
        sat: Sat,
        inscription_index: isize,
    ) -> Result<Option<InscriptionId>> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let sat_to_sequence_number = rtx.open_multimap_table(SAT_TO_SEQUENCE_NUMBER)?;

        if inscription_index < 0 {
            sat_to_sequence_number
                .get(&sat.n())?
                .nth_back((inscription_index + 1).abs_diff(0))
        } else {
            sat_to_sequence_number
                .get(&sat.n())?
                .nth(inscription_index.abs_diff(0))
        }
        .map(|result| {
            result
                .and_then(|sequence_number| {
                    let sequence_number = sequence_number.value();
                    sequence_number_to_inscription_entry
                        .get(sequence_number)
                        .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                })
                .map_err(|err| anyhow!(err.to_string()))
        })
        .transpose()
    }

    pub(crate) fn get_inscription_satpoint_by_id(
        &self,
        inscription_id: InscriptionId,
    ) -> Result<Option<SatPoint>> {
        let rtx = self.database.begin_read()?;

        let Some(sequence_number) = rtx
      .open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?
      .get(&inscription_id.store())?
      .map(|guard| guard.value())
    else {
      return Ok(None);
    };

        let satpoint = rtx
            .open_table(SEQUENCE_NUMBER_TO_SATPOINT)?
            .get(sequence_number)?
            .map(|satpoint| Entry::load(*satpoint.value()));

        Ok(satpoint)
    }

    pub(crate) fn get_inscription_by_id(
        &self,
        inscription_id: InscriptionId,
    ) -> Result<Option<Inscription>> {
        if !self.inscription_exists(inscription_id)? {
            return Ok(None);
        }

        Ok(self.get_transaction(inscription_id.txid)?.and_then(|tx| {
            ParsedEnvelope::from_transaction(&tx)
                .into_iter()
                .nth(inscription_id.index as usize)
                .map(|envelope| envelope.payload)
        }))
    }

    pub(crate) fn inscription_count(&self, txid: Txid) -> Result<u32> {
        let start = InscriptionId { index: 0, txid };

        let end = InscriptionId {
            index: u32::MAX,
            txid,
        };

        Ok(self
            .database
            .begin_read()?
            .open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?
            .range::<&InscriptionIdValue>(&start.store()..&end.store())?
            .count()
            .try_into()
            .unwrap())
    }

    pub(crate) fn inscription_exists(&self, inscription_id: InscriptionId) -> Result<bool> {
        Ok(self
            .database
            .begin_read()?
            .open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?
            .get(&inscription_id.store())?
            .is_some())
    }

    pub(crate) fn get_inscriptions_on_output_with_satpoints(
        &self,
        outpoint: OutPoint,
    ) -> Result<Vec<(SatPoint, InscriptionId)>> {
        let rtx = self.database.begin_read()?;
        let satpoint_to_sequence_number = rtx.open_multimap_table(SATPOINT_TO_SEQUENCE_NUMBER)?;
        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        Self::inscriptions_on_output(
            &satpoint_to_sequence_number,
            &sequence_number_to_inscription_entry,
            outpoint,
        )
    }

    pub(crate) fn get_inscriptions_on_output(
        &self,
        outpoint: OutPoint,
    ) -> Result<Vec<InscriptionId>> {
        Ok(self
            .get_inscriptions_on_output_with_satpoints(outpoint)?
            .iter()
            .map(|(_satpoint, inscription_id)| *inscription_id)
            .collect())
    }

    pub(crate) fn get_transaction(&self, txid: Txid) -> Result<Option<Transaction>> {
        if txid == self.genesis_block_coinbase_txid {
            return Ok(Some(self.genesis_block_coinbase_transaction.clone()));
        }

        if self.index_transactions {
            if let Some(transaction) = self
                .database
                .begin_read()?
                .open_table(TRANSACTION_ID_TO_TRANSACTION)?
                .get(&txid.store())?
            {
                return Ok(Some(consensus::encode::deserialize(transaction.value())?));
            }
        }

        self.client.get_raw_transaction(&txid, None).into_option()
    }

    pub(crate) fn find(&self, sat: Sat) -> Result<Option<SatPoint>> {
        let sat = sat.0;
        let rtx = self.begin_read()?;

        if rtx.block_count()? <= Sat(sat).height().n() {
            return Ok(None);
        }

        let outpoint_to_sat_ranges = rtx.0.open_table(OUTPOINT_TO_SAT_RANGES)?;

        for range in outpoint_to_sat_ranges.range::<&[u8; 36]>(&[0; 36]..)? {
            let (key, value) = range?;
            let mut offset = 0;
            for chunk in value.value().chunks_exact(11) {
                let (start, end) = SatRange::load(chunk.try_into().unwrap());
                if start <= sat && sat < end {
                    return Ok(Some(SatPoint {
                        outpoint: Entry::load(*key.value()),
                        offset: offset + sat - start,
                    }));
                }
                offset += end - start;
            }
        }

        Ok(None)
    }

    pub(crate) fn find_range(
        &self,
        range_start: Sat,
        range_end: Sat,
    ) -> Result<Option<Vec<FindRangeOutput>>> {
        let range_start = range_start.0;
        let range_end = range_end.0;
        let rtx = self.begin_read()?;

        if rtx.block_count()? < Sat(range_end - 1).height().n() + 1 {
            return Ok(None);
        }

        let Some(mut remaining_sats) = range_end.checked_sub(range_start) else {
      return Err(anyhow!("range end is before range start"));
    };

        let outpoint_to_sat_ranges = rtx.0.open_table(OUTPOINT_TO_SAT_RANGES)?;

        let mut result = Vec::new();
        for range in outpoint_to_sat_ranges.range::<&[u8; 36]>(&[0; 36]..)? {
            let (outpoint_entry, sat_ranges_entry) = range?;

            let mut offset = 0;
            for sat_range in sat_ranges_entry.value().chunks_exact(11) {
                let (start, end) = SatRange::load(sat_range.try_into().unwrap());

                if end > range_start && start < range_end {
                    let overlap_start = start.max(range_start);
                    let overlap_end = end.min(range_end);

                    result.push(FindRangeOutput {
                        start: overlap_start,
                        size: overlap_end - overlap_start,
                        satpoint: SatPoint {
                            outpoint: Entry::load(*outpoint_entry.value()),
                            offset: offset + overlap_start - start,
                        },
                    });

                    remaining_sats -= overlap_end - overlap_start;

                    if remaining_sats == 0 {
                        break;
                    }
                }
                offset += end - start;
            }
        }

        Ok(Some(result))
    }

    pub(crate) fn list(&self, outpoint: OutPoint) -> Result<Option<Vec<(u64, u64)>>> {
        Ok(self
            .database
            .begin_read()?
            .open_table(OUTPOINT_TO_SAT_RANGES)?
            .get(&outpoint.store())?
            .map(|outpoint| outpoint.value().to_vec())
            .map(|sat_ranges| {
                sat_ranges
                    .chunks_exact(11)
                    .map(|chunk| SatRange::load(chunk.try_into().unwrap()))
                    .collect::<Vec<(u64, u64)>>()
            }))
    }

    pub(crate) fn is_output_spent(&self, outpoint: OutPoint) -> Result<bool> {
        Ok(outpoint != OutPoint::null()
            && outpoint != self.settings.chain().genesis_coinbase_outpoint()
            && self
                .client
                .get_tx_out(&outpoint.txid, outpoint.vout, Some(true))?
                .is_none())
    }

    pub(crate) fn is_output_in_active_chain(&self, outpoint: OutPoint) -> Result<bool> {
        if outpoint == OutPoint::null() {
            return Ok(true);
        }

        if outpoint == self.settings.chain().genesis_coinbase_outpoint() {
            return Ok(true);
        }

        let Some(info) = self
      .client
      .get_raw_transaction_info(&outpoint.txid, None)
      .into_option()?
    else {
      return Ok(false);
    };

        if !info.in_active_chain.unwrap_or_default() {
            return Ok(false);
        }

        if usize::try_from(outpoint.vout).unwrap() >= info.vout.len() {
            return Ok(false);
        }

        Ok(true)
    }

    pub(crate) fn block_time(&self, height: Height) -> Result<Blocktime> {
        let height = height.n();

        let rtx = self.database.begin_read()?;

        let height_to_block_header = rtx.open_table(HEIGHT_TO_BLOCK_HEADER)?;

        if let Some(guard) = height_to_block_header.get(height)? {
            return Ok(Blocktime::confirmed(Header::load(*guard.value()).time));
        }

        let current = height_to_block_header
            .range(0..)?
            .next_back()
            .transpose()?
            .map(|(height, _header)| height)
            .map(|x| x.value())
            .unwrap_or(0);

        let expected_blocks = height.checked_sub(current).with_context(|| {
            format!("current {current} height is greater than sat height {height}")
        })?;

        Ok(Blocktime::Expected(
            Utc::now()
                .round_subsecs(0)
                .checked_add_signed(
                    chrono::Duration::try_seconds(10 * 60 * i64::from(expected_blocks))
                        .context("timestamp out of range")?,
                )
                .context("timestamp out of range")?,
        ))
    }

    pub(crate) fn get_inscriptions_paginated(
        &self,
        page_size: u32,
        page_index: u32,
    ) -> Result<(Vec<InscriptionId>, bool)> {
        let rtx = self.database.begin_read()?;

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let last = sequence_number_to_inscription_entry
            .iter()?
            .next_back()
            .map(|result| result.map(|(number, _entry)| number.value()))
            .transpose()?
            .unwrap_or_default();

        let start = last.saturating_sub(page_size.saturating_mul(page_index));

        let end = start.saturating_sub(page_size);

        let mut inscriptions = sequence_number_to_inscription_entry
            .range(end..=start)?
            .rev()
            .map(|result| result.map(|(_number, entry)| InscriptionEntry::load(entry.value()).id))
            .collect::<Result<Vec<InscriptionId>, StorageError>>()?;

        let more = u32::try_from(inscriptions.len()).unwrap_or(u32::MAX) > page_size;

        if more {
            inscriptions.pop();
        }

        Ok((inscriptions, more))
    }

    pub(crate) fn get_inscriptions_in_block(
        &self,
        block_height: u32,
    ) -> Result<Vec<InscriptionId>> {
        let rtx = self.database.begin_read()?;

        let height_to_last_sequence_number = rtx.open_table(HEIGHT_TO_LAST_SEQUENCE_NUMBER)?;
        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let Some(newest_sequence_number) = height_to_last_sequence_number
      .get(&block_height)?
      .map(|ag| ag.value())
    else {
      return Ok(Vec::new());
    };

        let oldest_sequence_number = height_to_last_sequence_number
            .get(block_height.saturating_sub(1))?
            .map(|ag| ag.value())
            .unwrap_or(0);

        (oldest_sequence_number..newest_sequence_number)
            .map(|num| match sequence_number_to_inscription_entry.get(&num) {
                Ok(Some(inscription_id)) => Ok(InscriptionEntry::load(inscription_id.value()).id),
                Ok(None) => Err(anyhow!(
                    "could not find inscription for inscription number {num}"
                )),
                Err(err) => Err(anyhow!(err)),
            })
            .collect::<Result<Vec<InscriptionId>>>()
    }

    pub(crate) fn get_highest_paying_inscriptions_in_block(
        &self,
        block_height: u32,
        n: usize,
    ) -> Result<(Vec<InscriptionId>, usize)> {
        let inscription_ids = self.get_inscriptions_in_block(block_height)?;

        let mut inscription_to_fee: Vec<(InscriptionId, u64)> = Vec::new();
        for id in &inscription_ids {
            inscription_to_fee.push((
                *id,
                self.get_inscription_entry(*id)?
                    .ok_or_else(|| anyhow!("could not get entry for inscription {id}"))?
                    .fee,
            ));
        }

        inscription_to_fee.sort_by_key(|(_, fee)| *fee);

        Ok((
            inscription_to_fee
                .iter()
                .map(|(id, _)| *id)
                .rev()
                .take(n)
                .collect(),
            inscription_ids.len(),
        ))
    }

    pub(crate) fn get_home_inscriptions(&self) -> Result<Vec<InscriptionId>> {
        Ok(self
            .database
            .begin_read()?
            .open_table(HOME_INSCRIPTIONS)?
            .iter()?
            .rev()
            .flat_map(|result| result.map(|(_number, id)| InscriptionId::load(id.value())))
            .collect())
    }

    pub(crate) fn get_feed_inscriptions(&self, n: usize) -> Result<Vec<(u32, InscriptionId)>> {
        Ok(self
            .database
            .begin_read()?
            .open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?
            .iter()?
            .rev()
            .take(n)
            .flat_map(|result| {
                result.map(|(number, entry)| {
                    (number.value(), InscriptionEntry::load(entry.value()).id)
                })
            })
            .collect())
    }

    pub fn inscription_info_benchmark(index: &Index, inscription_number: i32) {
        Self::inscription_info(index, query::Inscription::Number(inscription_number)).unwrap();
    }

    pub(crate) fn inscription_info(
        index: &Index,
        query: query::Inscription,
    ) -> Result<Option<InscriptionInfo>> {
        let rtx = index.database.begin_read()?;

        let sequence_number = match query {
            query::Inscription::Id(id) => {
                let inscription_id_to_sequence_number =
                    rtx.open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?;

                let sequence_number = inscription_id_to_sequence_number
                    .get(&id.store())?
                    .map(|guard| guard.value());

                drop(inscription_id_to_sequence_number);

                sequence_number
            }
            query::Inscription::Number(inscription_number) => {
                let inscription_number_to_sequence_number =
                    rtx.open_table(INSCRIPTION_NUMBER_TO_SEQUENCE_NUMBER)?;

                let sequence_number = inscription_number_to_sequence_number
                    .get(inscription_number)?
                    .map(|guard| guard.value());

                drop(inscription_number_to_sequence_number);

                sequence_number
            }
        };

        let Some(sequence_number) = sequence_number else {
      return Ok(None);
    };

        let sequence_number_to_inscription_entry =
            rtx.open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?;

        let entry = InscriptionEntry::load(
            sequence_number_to_inscription_entry
                .get(&sequence_number)?
                .unwrap()
                .value(),
        );

        let Some(transaction) = index.get_transaction(entry.id.txid)? else {
      return Ok(None);
    };

        let Some(inscription) = ParsedEnvelope::from_transaction(&transaction)
      .into_iter()
      .nth(entry.id.index as usize)
      .map(|envelope| envelope.payload)
    else {
      return Ok(None);
    };

        let satpoint = SatPoint::load(
            *rtx.open_table(SEQUENCE_NUMBER_TO_SATPOINT)?
                .get(sequence_number)?
                .unwrap()
                .value(),
        );

        let output =
            if satpoint.outpoint == unbound_outpoint() || satpoint.outpoint == OutPoint::null() {
                None
            } else {
                let Some(transaction) = index.get_transaction(satpoint.outpoint.txid)? else {
        return Ok(None);
      };

                transaction
                    .output
                    .into_iter()
                    .nth(satpoint.outpoint.vout.try_into().unwrap())
            };

        let previous = if let Some(n) = sequence_number.checked_sub(1) {
            Some(
                InscriptionEntry::load(
                    sequence_number_to_inscription_entry
                        .get(n)?
                        .unwrap()
                        .value(),
                )
                .id,
            )
        } else {
            None
        };

        let next = sequence_number_to_inscription_entry
            .get(sequence_number + 1)?
            .map(|guard| InscriptionEntry::load(guard.value()).id);

        let children = rtx
            .open_multimap_table(SEQUENCE_NUMBER_TO_CHILDREN)?
            .get(sequence_number)?
            .take(4)
            .map(|result| {
                result
                    .and_then(|sequence_number| {
                        sequence_number_to_inscription_entry
                            .get(sequence_number.value())
                            .map(|entry| InscriptionEntry::load(entry.unwrap().value()).id)
                    })
                    .map_err(|err| err.into())
            })
            .collect::<Result<Vec<InscriptionId>>>()?;

        let rune = if let Some(rune_id) = rtx
            .open_table(SEQUENCE_NUMBER_TO_RUNE_ID)?
            .get(sequence_number)?
        {
            let rune_id_to_rune_entry = rtx.open_table(RUNE_ID_TO_RUNE_ENTRY)?;
            let entry = rune_id_to_rune_entry.get(&rune_id.value())?.unwrap();
            Some(RuneEntry::load(entry.value()).spaced_rune())
        } else {
            None
        };

        let parent = match entry.parent {
            Some(parent) => Some(
                InscriptionEntry::load(
                    sequence_number_to_inscription_entry
                        .get(parent)?
                        .unwrap()
                        .value(),
                )
                .id,
            ),
            None => None,
        };

        let mut charms = entry.charms;

        if satpoint.outpoint == OutPoint::null() {
            Charm::Lost.set(&mut charms);
        }

        Ok(Some(InscriptionInfo {
            children,
            entry,
            parent,
            output,
            satpoint,
            inscription,
            previous,
            next,
            rune,
            charms,
        }))
    }

    pub(crate) fn get_inscription_entry(
        &self,
        inscription_id: InscriptionId,
    ) -> Result<Option<InscriptionEntry>> {
        let rtx = self.database.begin_read()?;

        let Some(sequence_number) = rtx
      .open_table(INSCRIPTION_ID_TO_SEQUENCE_NUMBER)?
      .get(&inscription_id.store())?
      .map(|guard| guard.value())
    else {
      return Ok(None);
    };

        let entry = rtx
            .open_table(SEQUENCE_NUMBER_TO_INSCRIPTION_ENTRY)?
            .get(sequence_number)?
            .map(|value| InscriptionEntry::load(value.value()));

        Ok(entry)
    }

    fn inscriptions_on_output<'a: 'tx, 'tx>(
        satpoint_to_sequence_number: &'a impl ReadableMultimapTable<&'static SatPointValue, u32>,
        sequence_number_to_inscription_entry: &'a impl ReadableTable<u32, InscriptionEntryValue>,
        outpoint: OutPoint,
    ) -> Result<Vec<(SatPoint, InscriptionId)>> {
        let start = SatPoint {
            outpoint,
            offset: 0,
        }
        .store();

        let end = SatPoint {
            outpoint,
            offset: u64::MAX,
        }
        .store();

        let mut inscriptions = Vec::new();

        for range in satpoint_to_sequence_number.range::<&[u8; 44]>(&start..=&end)? {
            let (satpoint, sequence_numbers) = range?;
            for sequence_number_result in sequence_numbers {
                let sequence_number = sequence_number_result?.value();
                let entry = sequence_number_to_inscription_entry
                    .get(sequence_number)?
                    .unwrap();
                inscriptions.push((
                    sequence_number,
                    SatPoint::load(*satpoint.value()),
                    InscriptionEntry::load(entry.value()).id,
                ));
            }
        }

        inscriptions.sort_by_key(|(sequence_number, _, _)| *sequence_number);

        Ok(inscriptions
            .into_iter()
            .map(|(_sequence_number, satpoint, inscription_id)| (satpoint, inscription_id))
            .collect())
    }
}
