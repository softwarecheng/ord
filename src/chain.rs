use {super::*, clap::ValueEnum};

#[derive(Default, ValueEnum, Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Chain {
    #[default]
    #[value(alias("main"))]
    Mainnet,
    #[value(alias("test"))]
    Testnet,
    Signet,
    Regtest,
    Testnet4,
}

impl Chain {
    pub(crate) fn network(self) -> Network {
        match self {
            Self::Mainnet => Network::Bitcoin,
            Self::Testnet => Network::Testnet,
            Self::Signet => Network::Signet,
            Self::Regtest => Network::Regtest,
            Self::Testnet4 => Network::Testnet4,
        }
    }

    pub(crate) fn default_rpc_port(self) -> u16 {
        match self {
            Self::Mainnet => 8332,
            Self::Regtest => 18443,
            Self::Signet => 38332,
            Self::Testnet => 18332,
            Self::Testnet4 => 28322,
        }
    }

    pub(crate) fn first_inscription_height(self) -> u32 {
        match self {
            Self::Mainnet => 767430,
            Self::Regtest => 0,
            Self::Signet => 112402,
            Self::Testnet => 2413343,
            Self::Testnet4 => 0,
        }
    }

    pub(crate) fn first_rune_height(self) -> u32 {
        SUBSIDY_HALVING_INTERVAL
            * match self {
                Self::Mainnet => 4,
                Self::Regtest => 0,
                Self::Signet => 0,
                Self::Testnet => 12,
                Self::Testnet4 => 0,
            }
    }

    pub(crate) fn jubilee_height(self) -> u32 {
        match self {
            Self::Mainnet => 824544,
            Self::Regtest => 110,
            Self::Signet => 175392,
            Self::Testnet => 2544192,
            Self::Testnet4 => 0,
        }
    }

    pub(crate) fn genesis_block(self) -> Block {
        bitcoint4::blockdata::constants::genesis_block(self.network())
    }

    pub(crate) fn genesis_coinbase_outpoint(self) -> OutPoint {
        OutPoint {
            txid: self.genesis_block().coinbase().unwrap().txid(),
            vout: 0,
        }
    }

    pub(crate) fn address_from_script(
        self,
        script: &Script,
    ) -> Result<Address, bitcoint4::address::Error> {
        Address::from_script(script, self.network())
    }

    pub(crate) fn join_with_data_dir(self, data_dir: impl AsRef<Path>) -> PathBuf {
        match self {
            Self::Mainnet => data_dir.as_ref().to_owned(),
            Self::Testnet => data_dir.as_ref().join("testnet3"),
            Self::Signet => data_dir.as_ref().join("signet"),
            Self::Regtest => data_dir.as_ref().join("regtest"),
            Self::Testnet4 => data_dir.as_ref().join("testnet4"),
        }
    }
}

impl Display for Chain {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Mainnet => "mainnet",
                Self::Regtest => "regtest",
                Self::Signet => "signet",
                Self::Testnet => "testnet",
                Self::Testnet4 => "testnet4",
            }
        )
    }
}

impl FromStr for Chain {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mainnet" => Ok(Self::Mainnet),
            "regtest" => Ok(Self::Regtest),
            "signet" => Ok(Self::Signet),
            "testnet" => Ok(Self::Testnet),
            _ => bail!("invalid chain `{s}`"),
        }
    }
}
