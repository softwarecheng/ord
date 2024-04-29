use super::*;

mod export;

#[derive(Debug, Parser)]
pub(crate) enum OrdxSubcommand {
  #[command(about = "Export inscription and output json file with every block for ordx")]
  Export(export::Export),
}

impl OrdxSubcommand {
  pub(crate) fn run(self, settings: Settings) -> SubcommandResult {
    match self {
      Self::Export(export) => export.run(settings),
    }
  }
}
