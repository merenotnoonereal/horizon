//! The `player_destroy_item` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0250

pub trait Hooks {
    /// The player's item was used up or broke. `original` is the complete
    /// stack before the use or durability loss, including count, damage and
    /// components. `hand` is the hand it was in, or None for worn equipment
    /// and ammunition consumed from the inventory. `player` and
    /// `player_entity_id` identify the acting player, including the local host.
    /// Listen only, after consumption or the break has applied. Both logical
    /// sides hear their own actions; a client hears its prediction or completed
    /// use, not another player's inventory updates. A partial consumption, a
    /// refused action and creative use that preserves the stack do not fire.
    /// Native and wasm listeners receive the same immutable snapshot and HostSide.
    #[hook(
        side = "both",
        version = 2,
        contract = "listen",
        neoforge = "PlayerDestroyItemEvent"
    )]
    fn player_destroy_item();
}
