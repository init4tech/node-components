use ajj::serde_json;
use eyre::WrapErr;
use reth::providers::{ProviderFactory, providers::ProviderNodeTypes};
use reth_db::{Database, TableViewer, mdbx, table::Table};
use reth_db_common::{DbTool, ListFilter};
use signet_node_types::Pnt;
use std::sync::{Arc, OnceLock};
use tracing::instrument;

/// Modeled on the `Command` struct from `reth/crates/cli/commands/src/db/list.rs`
#[derive(Debug, serde::Deserialize)]
pub(crate) struct DbArgs(
    /// The table name
    String, // 0
    /// Skip first N entries
    #[serde(default)]
    usize, // 1
    /// How many items to take from the walker
    #[serde(default)]
    Option<usize>, // 2
    /// Search parameter for both keys and values. Prefix it with `0x` to search for binary data,
    /// and text otherwise.
    ///
    /// ATTENTION! For compressed tables (`Transactions` and `Receipts`), there might be
    /// missing results since the search uses the raw uncompressed value from the database.
    #[serde(default)]
    Option<String>, // 3
);

impl DbArgs {
    /// Get the table name.
    pub(crate) fn table_name(&self) -> &str {
        &self.0
    }

    /// Parse the table name into a [`reth_db::Tables`] enum.
    pub(crate) fn table(&self) -> Result<reth_db::Tables, String> {
        self.table_name().parse()
    }

    /// Get the skip value.
    pub(crate) const fn skip(&self) -> usize {
        self.1
    }

    /// Get the length value.
    pub(crate) fn len(&self) -> usize {
        self.2.unwrap_or(5)
    }

    /// Get the search value.
    pub(crate) fn search(&self) -> Vec<u8> {
        self.3
            .as_ref()
            .map(|search| {
                if let Some(search) = search.strip_prefix("0x") {
                    return alloy::primitives::hex::decode(search).unwrap();
                }
                search.as_bytes().to_vec()
            })
            .unwrap_or_default()
    }

    /// Generate [`ListFilter`] from command.
    pub(crate) fn list_filter(&self) -> ListFilter {
        ListFilter {
            skip: self.skip(),
            len: self.len(),
            search: self.search(),
            min_row_size: 0,
            min_key_size: 0,
            min_value_size: 0,
            reverse: false,
            only_count: false,
        }
    }
}

pub(crate) struct ListTableViewer<'a, 'b, N: Pnt> {
    pub(crate) factory: &'b ProviderFactory<N>,
    pub(crate) args: &'a DbArgs,

    pub(crate) output: OnceLock<Box<serde_json::value::RawValue>>,
}

impl<'a, 'b, N: Pnt> ListTableViewer<'a, 'b, N> {
    /// Create a new `ListTableViewer`.
    pub(crate) fn new(factory: &'b ProviderFactory<N>, args: &'a DbArgs) -> Self {
        Self { factory, args, output: Default::default() }
    }

    /// Take the output if it has been initialized, otherwise return `None`.
    pub(crate) fn take_output(self) -> Option<Box<serde_json::value::RawValue>> {
        self.output.into_inner()
    }
}

impl<N: Pnt + ProviderNodeTypes<DB = Arc<mdbx::DatabaseEnv>>> TableViewer<()>
    for ListTableViewer<'_, '_, N>
{
    type Error = eyre::Report;

    #[instrument(skip(self), err)]
    fn view<T: Table>(&self) -> eyre::Result<()> {
        let tool = DbTool { provider_factory: self.factory.clone() };

        self.factory.db_ref().view(|tx| {
            let table_db =
                tx.inner().open_db(Some(self.args.table_name())).wrap_err("Could not open db.")?;
            let stats = tx
                .inner()
                .db_stat(table_db.dbi())
                .wrap_err(format!("Could not find table: {}", stringify!($table)))?;
            let total_entries = stats.entries();
            let final_entry_idx = total_entries.saturating_sub(1);
            eyre::ensure!(
                self.args.skip() >= final_entry_idx,
                "Skip value {} is greater than total entries {}",
                self.args.skip(),
                total_entries
            );

            let list_filter = self.args.list_filter();

            let (list, _) = tool.list::<T>(&list_filter)?;

            let json =
                serde_json::value::to_raw_value(&list).wrap_err("Failed to serialize list")?;

            self.output.get_or_init(|| json);

            Ok(())
        })??;

        Ok(())
    }
}

// Some code in this file is adapted from github.com/paradigmxyz/reth.
//
// Particularly the `reth/crates/cli/commands/src/db/list.rs` file. It is
// reproduced here under the terms of the MIT license,
//
// The MIT License (MIT)
//
// Copyright (c) 2022-2025 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
