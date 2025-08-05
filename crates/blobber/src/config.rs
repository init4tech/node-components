use init4_bin_base::utils::from_env::FromEnv;
use std::borrow::Cow;

/// Configuration for the block extractor.
#[derive(Debug, Clone, serde::Deserialize, FromEnv)]
#[serde(rename_all = "camelCase")]
pub struct BlobFetcherConfig {
    /// URL of the blob explorer.
    #[from_env(var = "BLOB_EXPLORER_URL", desc = "URL of the blob explorer", infallible)]
    blob_explorer_url: Cow<'static, str>,

    /// Consensus layer RPC URL
    #[from_env(var = "SIGNET_CL_URL", desc = "Consensus layer URL", infallible, optional)]
    cl_url: Option<Cow<'static, str>>,

    /// The Pylon node URL
    #[from_env(var = "SIGNET_PYLON_URL", desc = "Pylon node URL", infallible, optional)]
    pylon_url: Option<Cow<'static, str>>,
}

impl BlobFetcherConfig {
    /// Create a new `BlobFetcherConfig` with default values.
    pub const fn new(blob_explorer_url: Cow<'static, str>) -> Self {
        Self { blob_explorer_url, cl_url: None, pylon_url: None }
    }

    /// Get the blob explorer URL.
    pub fn set_blob_explorer_url(&mut self, blob_explorer_url: Cow<'static, str>) {
        self.blob_explorer_url = blob_explorer_url;
    }

    /// Get the blob explorer URL.
    pub fn set_cl_url(&mut self, cl_url: Cow<'static, str>) {
        self.cl_url = Some(cl_url);
    }

    /// Set the Pylon URL.
    pub fn set_pylon_url(&mut self, pylon_url: Cow<'static, str>) {
        self.pylon_url = Some(pylon_url);
    }

    /// Create a new `BlobFetcherConfig` with the provided CL URL, Pylon URL,
    pub fn cl_url(&self) -> Option<&str> {
        self.cl_url.as_deref()
    }

    /// Get the Pylon URL.
    pub fn pylon_url(&self) -> Option<&str> {
        self.pylon_url.as_deref()
    }

    /// Get the blob explorer URL.
    pub fn blob_explorer_url(&self) -> &str {
        &self.blob_explorer_url
    }
}
