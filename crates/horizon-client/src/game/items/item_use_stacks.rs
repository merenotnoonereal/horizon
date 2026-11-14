//! Stack snapshots for block-use criteria and item-destruction notices.
use super::*;
use horizon_client::items::ItemInfo;

#[derive(Default)]
pub(super) struct ItemUseStacks {
    pub(super) block_originals: Vec<(Hand, Option<ItemStack>)>,
    notices: Vec<(Hand, bool)>,
}

impl Game<'_> {
    /// A slot's hand, or no hand for armour and inventory ammunition.
    pub(super) fn item_slot_hand(&self, slot: usize) -> Option<Hand> {
        if slot == shield::OFFHAND_SLOT {
            Some(Hand::Off)
        } else if slot == self.selected {
            Some(Hand::Main)
        } else {
            None
        }
    }

    /// Report an applied destruction and settle enclosing uses of the same hand.
    pub(super) fn player_item_destroyed(&mut self, original: &ItemStack, hand: Option<Hand>) {
        if original.count == 0
            || &*original.item == "minecraft:air"
            || !mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
        {
            return;
        }
        if let Some(hand) = hand {
            for (acting, notified) in &mut self.item_use_stacks.notices {
                if *acting == hand {
                    *notified = true;
                }
            }
        }
        let (player, player_entity_id) = self.native_clicker();
        self.native_player_destroy_item(player, player_entity_id, original, hand);
    }

    /// Consume from a player slot after the caller's creative-mode gate.
    /// Notify before a use remainder replaces the now-empty stack.
    pub(super) fn consume_player_slot(&mut self, slot: usize, amount: u32) {
        let original = mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem)
            .then(|| self.stack_in(slot).cloned())
            .flatten();
        let hand = self.item_slot_hand(slot);
        let edit = |cell: &mut Option<ItemStack>| {
            if let Some(stack) = cell.as_mut() {
                stack.count = stack.count.saturating_sub(amount);
                if stack.count == 0 {
                    *cell = None;
                }
            }
        };
        if slot == self.held_slot() {
            self.edit_block_use_stack(edit);
        } else {
            edit(self.stack_in_mut(slot));
        }
        if self.stack_in(slot).is_none_or(|stack| stack.count == 0)
            && let Some(original) = original
        {
            self.player_item_destroyed(&original, hand);
        }
    }

    pub(super) fn consume_held(&mut self) {
        if !self.creative {
            self.consume_player_slot(self.held_slot(), 1);
        }
    }

    /// Catch direct hand edits by item/entity implementations, including Parts.
    /// Nested uses and ordinary consume/break helpers share a single notice.
    pub(super) fn with_item_destroy_notice<R>(
        &mut self,
        hand: Hand,
        action: impl FnOnce(&mut Self) -> R,
    ) -> R {
        if !mod_api::__private::listened(mod_api::HookId::PlayerDestroyItem) {
            return action(self);
        }
        let original = self.with_hand(hand, |g| g.held().cloned());
        let depth = self.item_use_stacks.notices.len();
        self.item_use_stacks.notices.push((hand, false));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(self)));
        let notified = self.item_use_stacks.notices[depth].1;
        self.item_use_stacks.notices.truncate(depth);
        match result {
            Ok(value) => {
                if !notified
                    && self.with_hand(hand, |g| g.held().is_none_or(|s| s.count == 0))
                    && let Some(original) = original
                {
                    self.player_item_destroyed(&original, Some(hand));
                }
                value
            }
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    pub(in crate::game) fn use_hand(&mut self, hit: Option<&Hit>) -> Result<bool> {
        self.with_item_destroy_notice(self.hand, |g| g.use_hand_inner(hit))
    }

    pub(super) fn interact_entity(&mut self, target: Option<InteractTarget>) -> Result<bool> {
        self.with_item_destroy_notice(self.hand, |g| g.interact_entity_inner(target))
    }

    pub(super) fn remote_use_item(&mut self, hand: Hand) -> bool {
        self.with_item_destroy_notice(hand, |g| g.remote_use_item_inner(hand))
    }

    pub(super) fn remote_use_item_on(&mut self, hand: Hand) -> bool {
        self.with_item_destroy_notice(hand, |g| g.remote_use_item_on_inner(hand))
    }

    pub(super) fn remote_use_entity(&mut self, hand: Hand) -> bool {
        self.with_item_destroy_notice(hand, |g| g.remote_use_entity_inner(hand))
    }

    pub(super) fn complete_using_item(&mut self, slot: usize, stack: &ItemStack, info: &ItemInfo) {
        let hand = self.item_slot_hand(slot).unwrap_or(self.hand);
        self.with_item_destroy_notice(hand, |g| g.complete_using_item_inner(slot, stack, info));
    }

    pub(super) fn remote_use_item_finished(
        &mut self,
        slot: usize,
        stack: &ItemStack,
        info: &ItemInfo,
    ) {
        let hand = self.item_slot_hand(slot).unwrap_or(self.hand);
        self.with_item_destroy_notice(hand, |g| {
            g.remote_use_item_finished_inner(slot, stack, info)
        });
    }
}
