// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use rezzer_model::Registry;

fn main() -> std::process::ExitCode {
    rezzer_cli::main(registry())
}

/// The readers of this build, chosen by the `fmt-*` cargo features.
fn registry() -> Registry {
    #[allow(unused_mut)]
    let mut registry = Registry::new();
    #[cfg(feature = "fmt-text")]
    registry.register(Box::new(rezzer_read_text::TextReader));
    #[cfg(feature = "fmt-markdown")]
    registry.register(Box::new(rezzer_read_markdown::MarkdownReader));
    #[cfg(feature = "fmt-csv")]
    registry.register(Box::new(rezzer_read_csv::CsvReader));
    registry
}
