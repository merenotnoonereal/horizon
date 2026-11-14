//! A guest's result take is one action, including every trade of a quick move. This collect
//! hook has no veto or stale answer: its validated offer replaces the live offer and stands.
use super::{Game, decided_action::Epoch, transactions::After};
use horizon_client::{
    advancements::Advancement,
    lan::conn::transaction_packets::{Packets, Scope},
    merchant::MerchantOffer,
    scoreboard::Scoreboard,
};
use horizon_protocol::packets::play;
use std::{cell::Cell, collections::HashMap};

thread_local! {
    static INVALID_OFFER: Cell<bool> = const { Cell::new(false) };
}

pub(in crate::game) fn invalid_offer() {
    INVALID_OFFER.set(true);
}

// Keep typed snapshots in closures so no storage's private representation escapes its owner.
type Restore = Box<dyn FnOnce(&mut Game<'_>)>;
#[derive(Default)]
pub(in crate::game) struct Journal {
    guests: HashMap<u32, Restore>,
    board: Option<Scoreboard>,
    map_last_id: Option<Option<i32>>,
}
impl Journal {
    pub(in crate::game) fn merge(&mut self, child: Self) {
        for (conn, kept) in child.guests {
            self.guests.entry(conn).or_insert(kept);
        }
        if self.board.is_none() {
            self.board = child.board;
        }
        if self.map_last_id.is_none() {
            self.map_last_id = child.map_last_id;
        }
    }
    pub(in crate::game) fn rollback(self, game: &mut Game<'_>) {
        for restore in self.guests.into_values() {
            restore(game);
        }
        if let Some(board) = self.board {
            game.score.board = board;
        }
        if let Some(last_id) = self.map_last_id {
            game.maps.last_id = last_id;
        }
    }
}

pub(in crate::game) enum Effect {
    Packets(Packets),
    Orb([f64; 3], u32),
    Drops(u32, Vec<horizon_client::items::ItemStack>),
    Reward(u128, Advancement),
    Maps(Vec<(i32, [i32; 3], u8, String)>),
}

impl Game<'_> {
    pub(super) fn lan_merchant_click(&mut self, conn: u32, p: &play::ContainerClick) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::TradeWithVillager)
            || p.slot != 2
            || !matches!(p.input, 0 | 1 | 2 | 4)
        {
            return self.lan_merchant_click_plain(conn, p);
        }
        let open = self
            .lan
            .as_ref()
            .and_then(|h| h.play.get(&conn))
            .and_then(|p| p.merchant.as_ref())
            .is_some_and(|m| m.window == p.container && m.merchant.mob.is_some());
        let Some((actor, route)) = open
            .then(|| Some((self.guest_actor(conn)?, self.guest_route(conn)?)))
            .flatten()
        else {
            return self.lan_merchant_click_plain(conn, p);
        };
        let handled = Cell::new(true);
        let applied = self.decided_action(
            "trade_with_villager",
            route,
            actor,
            Epoch::Off,
            |_| true,
            |game| {
                game.journal_merchant_take(conn);
                handled.set(game.lan_merchant_click_plain(conn, p));
                crate::game::transactions::decision::Reply::Allow
            },
            |_| {},
        );
        if !applied {
            // The client predicted a take. The restored menu, including the offer answer,
            // goes back even on listener failure, with the same handled result.
            self.resend_merchant_take(conn);
        }
        handled.get()
    }

    fn journal_merchant_take(&mut self, conn: u32) {
        self.journal_guest_click(conn);
        let Some(level) = self.transactions.last() else {
            return;
        };
        if level.trades.guests.contains_key(&conn) {
            return;
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let Some(g) = host.hub.guest(conn) else {
            return;
        };
        let Some(play) = host.play.get(&conn) else {
            return;
        };
        let Some(merchant) = play.merchant.clone() else {
            return;
        };
        let stats = play.stats.clone();
        let progress = g.advancements.clone();
        let changed = g.advancements_changed.clone();
        let unlocked = g.recipes_unlocked.clone();
        let book = g.recipe_book.clone();
        let mob = merchant
            .merchant
            .mob
            .and_then(|id| self.mobs.iter().find(|m| m.id == id))
            .map(|m| {
                (
                    m.id,
                    m.random.clone(),
                    m.ambient_timer,
                    m.extra.clone(),
                    m.needs_sync,
                )
            });
        let board = self.score.board.clone();
        let level = self.transactions.last_mut().expect("the action is open");
        level.trades.board.get_or_insert(board);
        level.trades.map_last_id.get_or_insert(self.maps.last_id);
        level.trades.guests.insert(
            conn,
            Box::new(move |game| {
                if let Some(host) = game.lan.as_mut() {
                    let play = host.play.entry(conn).or_default();
                    play.merchant = Some(merchant);
                    play.stats = stats;
                    if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) {
                        host.hub.guests.update(gi, |g| {
                            g.advancements = progress;
                            g.advancements_changed = changed;
                            g.recipes_unlocked = unlocked;
                            g.recipe_book = book;
                        });
                    }
                }
                if let Some((id, random, timer, extra, sync)) = mob
                    && let Some(m) = game.mobs.iter_mut().find(|m| m.id == id)
                {
                    m.random = random;
                    m.ambient_timer = timer;
                    m.extra = extra;
                    m.needs_sync = sync;
                }
            }),
        );
        if level.packets.is_none() {
            level.packets = Some(Scope::begin());
        }
    }

    fn resend_merchant_take(&mut self, conn: u32) {
        let Some(mut gm) = self
            .lan
            .as_mut()
            .and_then(|h| h.play.get_mut(&conn))
            .and_then(|p| p.merchant.take())
        else {
            return;
        };
        self.lan_send_offers(conn, &mut gm, true);
        self.lan_send_merchant_content(conn, &mut gm);
        if let Some(host) = self.lan.as_mut() {
            host.play.entry(conn).or_default().merchant = Some(gm);
        }
    }

    pub(super) fn merchant_take_active(&self) -> bool {
        self.transactions
            .iter()
            .any(|l| !l.trades.guests.is_empty())
    }

    /// Keep the existing offer clamps; a non-finite answer cannot become a live price.
    pub(super) fn merchant_trade_answer(&mut self, id: u32, kind: &str, offer: &mut MerchantOffer) {
        if !mod_api::__private::listened(mod_api::HookId::TradeWithVillager) {
            return self.native_trade_with_villager(id, kind, offer);
        }
        INVALID_OFFER.set(false);
        self.native_trade_with_villager(id, kind, offer);
        assert!(
            !INVALID_OFFER.replace(false),
            "invalid merchant offer answer"
        );
        assert!(
            offer.price_multiplier.is_finite(),
            "non-finite merchant price multiplier"
        );
        offer.uses = offer.uses.max(0);
        offer.max_uses = offer.max_uses.max(1);
        offer.xp = offer.xp.max(0);
    }

    pub(super) fn merchant_maps(&mut self, made: Vec<(i32, [i32; 3], u8, String)>) {
        if self.merchant_take_active() {
            if !made.is_empty() {
                self.maps.last_id = made.iter().map(|m| m.0).chain(self.maps.last_id).max();
                self.after_commit(After::Trade(Box::new(Effect::Maps(made))));
            }
        } else {
            self.register_exploration_maps(made);
        }
    }

    pub(super) fn merchant_drops(
        &mut self,
        conn: u32,
        stacks: Vec<horizon_client::items::ItemStack>,
    ) {
        if self.merchant_take_active() {
            if !stacks.is_empty() {
                self.after_commit(After::Trade(Box::new(Effect::Drops(conn, stacks))));
            }
        } else if let Some(gi) = self
            .lan
            .as_ref()
            .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
        {
            self.drop_guest_items(gi, stacks);
        }
    }

    pub(super) fn merchant_orb(&mut self, at: [f64; 3], value: u32) {
        if self.merchant_take_active() {
            self.after_commit(After::Trade(Box::new(Effect::Orb(at, value))));
        } else {
            self.add_orb(at, value);
        }
    }
    pub(super) fn merchant_sound(&mut self, sound: &'static str, at: [f64; 3], pitch: f32) {
        if self.merchant_take_active() {
            self.defer_effect(super::transactions::DeferredEffect::Sound {
                sound,
                at,
                volume: 1.0,
                pitch,
            });
        } else {
            self.audio.play(sound, Some(at), 1.0, pitch);
        }
    }

    // The criterion and its listener run tentatively. Rewards can spawn entities and run
    // arbitrary functions, so their publication belongs to the successful root commit.
    pub(super) fn defer_merchant_reward(&mut self, advancement: &Advancement) -> bool {
        if !self.merchant_take_active() {
            return false;
        }
        let uuid = self.native_acting_id();
        self.after_commit(After::Trade(Box::new(Effect::Reward(
            uuid,
            advancement.clone(),
        ))));
        true
    }
    pub(in crate::game) fn commit_merchant_effect(&mut self, effect: Effect) {
        match effect {
            Effect::Packets(packets) => packets.publish(),
            Effect::Maps(made) => self.register_exploration_maps(made),
            Effect::Orb(at, value) => self.add_orb(at, value),
            Effect::Drops(conn, stacks) => {
                if let Some(gi) = self
                    .lan
                    .as_ref()
                    .and_then(|h| h.hub.guests.iter().position(|g| g.conn == conn))
                {
                    self.drop_guest_items(gi, stacks);
                }
            }
            Effect::Reward(uuid, advancement) => {
                self.native_as_player(uuid, |game| game.commit_merchant_reward(&advancement));
            }
        }
    }
}
