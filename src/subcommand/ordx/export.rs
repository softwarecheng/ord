use super::*;

#[derive(Debug, Parser)]
pub(crate) struct Export {
  #[arg(long, help = "export <Dir>")]
  dir: String,
  #[arg(long, help = "export <CHAIN>")]
  chain: Chain,
  #[arg(long, help = "export <ord first height>")]
  first_height: u32,
}

impl Export {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    index.update()?;
    index.export_ordx(&self.dir, self.chain, self.first_height)?;

    Ok(None)
  }
}
