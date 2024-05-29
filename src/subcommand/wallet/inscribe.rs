use super::*;

#[derive(Debug, Parser)]
#[clap(
  group = ArgGroup::new("source")
      .required(true)
      .args(&["file", "batch"]),
)]
pub(crate) struct Inscribe {
  #[arg(
    long,
    help = "Inscribe multiple inscriptions defined in a yaml <BATCH_FILE>.",
    conflicts_with_all = &[
      "cbor_metadata", "delegate", "destination", "file", "json_metadata", "metaprotocol",
      "parent", "postage", "reinscribe", "sat", "satpoint"
    ]
  )]
  pub(crate) batch: Option<PathBuf>,
  #[arg(
    long,
    help = "Include CBOR in file at <METADATA> as inscription metadata",
    conflicts_with = "json_metadata"
  )]
  pub(crate) cbor_metadata: Option<PathBuf>,
  #[arg(
    long,
    help = "Use <COMMIT_FEE_RATE> sats/vbyte for commit transaction.\nDefaults to <FEE_RATE> if unset."
  )]
  pub(crate) commit_fee_rate: Option<FeeRate>,
  #[arg(long, help = "Compress inscription content with brotli.")]
  pub(crate) compress: bool,
  #[arg(long, help = "Delegate inscription content to <DELEGATE>.")]
  pub(crate) delegate: Option<InscriptionId>,
  #[arg(long, help = "Send inscription to <DESTINATION>.")]
  pub(crate) destination: Option<Address<NetworkUnchecked>>,
  #[arg(long, help = "Don't sign or broadcast transactions.")]
  pub(crate) dry_run: bool,
  #[arg(long, help = "Use fee rate of <FEE_RATE> sats/vB.")]
  pub(crate) fee_rate: FeeRate,
  #[arg(long, help = "Inscribe sat with contents of <FILE>.")]
  pub(crate) file: Option<PathBuf>,
  #[arg(
    long,
    help = "Include JSON in file at <METADATA> converted to CBOR as inscription metadata",
    conflicts_with = "cbor_metadata"
  )]
  pub(crate) json_metadata: Option<PathBuf>,
  #[clap(long, help = "Set inscription metaprotocol to <METAPROTOCOL>.")]
  pub(crate) metaprotocol: Option<String>,
  #[arg(long, alias = "nobackup", help = "Do not back up recovery key.")]
  pub(crate) no_backup: bool,
  #[arg(
    long,
    alias = "nolimit",
    help = "Do not check that transactions are equal to or below the MAX_STANDARD_TX_WEIGHT of 400,000 weight units. Transactions over this limit are currently nonstandard and will not be relayed by bitcoind in its default configuration. Do not use this flag unless you understand the implications."
  )]
  pub(crate) no_limit: bool,
  #[clap(long, help = "Make inscription a child of <PARENT>.")]
  pub(crate) parent: Option<InscriptionId>,
  #[arg(
    long,
    help = "Amount of postage to include in the inscription. Default `10000sat`."
  )]
  pub(crate) postage: Option<Amount>,
  #[clap(long, help = "Allow reinscription.")]
  pub(crate) reinscribe: bool,
  #[arg(long, help = "Inscribe <SAT>.", conflicts_with = "satpoint")]
  pub(crate) sat: Option<Sat>,
  #[arg(long, help = "Inscribe <SATPOINT>.", conflicts_with = "sat")]
  pub(crate) satpoint: Option<SatPoint>,
}

impl Inscribe {
  pub(crate) fn run(self, wallet: Wallet) -> SubcommandResult {
    let metadata = Inscribe::parse_metadata(self.cbor_metadata, self.json_metadata)?;

    let utxos = wallet.utxos();

    let mut locked_utxos = wallet.locked_utxos().clone();

    let runic_utxos = wallet.get_runic_outputs()?;

    let chain = wallet.chain();

    let postages;
    let destinations;
    let inscriptions;
    let mode;
    let parent_info;
    let reinscribe;
    let reveal_satpoints;

    let satpoint = match (self.file, self.batch) {
      (Some(file), None) => {
        parent_info = wallet.get_parent_info(self.parent)?;

        postages = vec![self.postage.unwrap_or(TARGET_POSTAGE)];

        if let Some(delegate) = self.delegate {
          ensure! {
            wallet.inscription_exists(delegate)?,
            "delegate {delegate} does not exist"
          }
        }

        inscriptions = vec![Inscription::from_file(
          chain,
          self.compress,
          self.delegate,
          metadata,
          self.metaprotocol,
          self.parent,
          file,
          None,
        )?];

        mode = Mode::SeparateOutputs;

        reinscribe = self.reinscribe;

        reveal_satpoints = Vec::new();

        destinations = vec![match self.destination.clone() {
          Some(destination) => destination.require_network(chain.network())?,
          None => wallet.get_change_address()?,
        }];

        if let Some(sat) = self.sat {
          Some(wallet.find_sat_in_outputs(sat)?)
        } else {
          self.satpoint
        }
      }
      (None, Some(batch)) => {
        let batchfile = Batchfile::load(&batch)?;

        parent_info = wallet.get_parent_info(batchfile.parent)?;

        (inscriptions, reveal_satpoints, postages, destinations) = batchfile.inscriptions(
          &wallet,
          utxos,
          parent_info.as_ref().map(|info| info.tx_out.value),
          self.compress,
        )?;

        locked_utxos.extend(
          reveal_satpoints
            .iter()
            .map(|(satpoint, txout)| (satpoint.outpoint, txout.clone())),
        );

        mode = batchfile.mode;

        reinscribe = batchfile.reinscribe;

        if let Some(sat) = batchfile.sat {
          Some(wallet.find_sat_in_outputs(sat)?)
        } else {
          batchfile.satpoint
        }
      }
      _ => unreachable!(),
    };

    Batch {
      commit_fee_rate: self.commit_fee_rate.unwrap_or(self.fee_rate),
      destinations,
      dry_run: self.dry_run,
      inscriptions,
      mode,
      no_backup: self.no_backup,
      no_limit: self.no_limit,
      parent_info,
      postages,
      reinscribe,
      reveal_fee_rate: self.fee_rate,
      reveal_satpoints,
      satpoint,
    }
    .inscribe(
      &locked_utxos.into_keys().collect(),
      runic_utxos,
      utxos,
      &wallet,
    )
  }

  fn parse_metadata(cbor: Option<PathBuf>, json: Option<PathBuf>) -> Result<Option<Vec<u8>>> {
    if let Some(path) = cbor {
      let cbor = fs::read(path)?;
      let _value: Value = ciborium::from_reader(Cursor::new(cbor.clone()))
        .context("failed to parse CBOR metadata")?;

      Ok(Some(cbor))
    } else if let Some(path) = json {
      let value: serde_json::Value =
        serde_json::from_reader(File::open(path)?).context("failed to parse JSON metadata")?;
      let mut cbor = Vec::new();
      ciborium::into_writer(&value, &mut cbor)?;

      Ok(Some(cbor))
    } else {
      Ok(None)
    }
  }
}
