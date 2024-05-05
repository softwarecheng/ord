use super::*;

#[derive(Debug, Parser)]
pub(crate) struct Export {
  #[arg(long, help = "export <Dir>")]
  dir: String,
  #[arg(long, help = "export <CHAIN>")]
  chain: Chain,
}

impl Export {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    index.update()?;
    index.export_ordx(&self.dir, self.chain)?;

    Ok(None)
  }
}
