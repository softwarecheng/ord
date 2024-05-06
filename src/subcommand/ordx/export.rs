use super::*;

#[derive(Debug, Parser)]
pub(crate) struct Export {
  #[arg(long, help = "Write export to filename path")]
  filename: String,
}

impl Export {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    let index = Index::open(&settings)?;

    index.update()?;
    index.export_ordx(
      &self.filename,
      settings.chain(),
      settings.first_inscription_height(),
    )?;

    Ok(None)
  }
}
