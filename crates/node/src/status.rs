/// Items that can be sent via the status channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// Node is booting.
    Booting,
    /// Node's current height.
    AtHeight(u64),
}
