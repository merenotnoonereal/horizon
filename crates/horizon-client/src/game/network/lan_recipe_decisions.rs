//! Recipe placement is one ordered menu plan, including all lent containers. Nothing is
//! copied back until every callback has returned and the coordinator validates the read epoch.
//! The unsubscribed routes in lan_crafting retain their ordinary path.
use super::{
    Game,
    decided_action::Epoch,
    lan_crafting::{load_player, store_player},
};
use crate::game::transactions::{
    After,
    decision::{Reply, engine::Target},
};
use horizon_client::{
    containers::BlockEntity,
    furnace::FurnaceCooking,
    items::ItemStack,
    lan::menu::ModCraft,
    menu::{Env, Kind, Menu},
};
use horizon_protocol::packets::play::Clientbound;
use std::{cell::RefCell, rc::Rc};

type Pos = (i32, i32, i32);
type Source = (Pos, Vec<Option<ItemStack>>);

enum Destination {
    Inventory,
    Table {
        state: i32,
        pos: [i32; 3],
    },
    Mod {
        pos: Pos,
        items: Vec<Option<ItemStack>>,
    },
    Furnace {
        pos: Pos,
        items: [Option<ItemStack>; 3],
    },
}

struct Placement {
    menu: Box<Menu>,
    destination: Destination,
    sources: Vec<Source>,
}

/// Replies and corrections use the existing root-commit effect queue.
pub(in crate::game) enum Publication {
    Sync {
        conn: u32,
        generation: u64,
        container: i32,
    },
    Packet(horizon_client::lan::PlayerSender, Box<Clientbound>),
    Ghost {
        conn: u32,
        generation: u64,
        container: i32,
        recipe: String,
    },
}

impl Game<'_> {
    /// Whether this packet names a placement owner, before admitting or dispatching an action.
    pub(super) fn recipe_window_available(&self, gi: usize, container: i32) -> bool {
        let Some(host) = self.lan.as_ref() else {
            return false;
        };
        let Some(g) = host.hub.guests.get(gi) else {
            return false;
        };
        container == 0
            || self.lan_mod_crafting_window(gi, container).is_some()
            || host.menus.get(&g.conn).is_some_and(|p| p.crafting.table.as_ref().is_some_and(|(w, ..)| *w == container))
            || g.menu.as_ref().filter(|m| m.window == container).is_some_and(|m| {
                matches!(m.slots, horizon_client::lan::menu::Slots::Block(pos) if matches!(self.world.block_entities.get(&pos), Some(BlockEntity::Furnace(_))))
            })
    }

    /// Constant-time subscriber queries; no menu, read set or transaction on the ordinary path.
    pub(super) fn recipe_listened(&self, gi: usize, container: i32, book: bool) -> bool {
        use mod_api::{__private::listened, HookId as H};
        (book && listened(H::PlayerRecipeBookClick))
            || listened(H::IngredientTest)
            || mod_api::__private::has_part_hook(H::MenuSlotRules)
            || (self.lan_mod_crafting_window(gi, container).is_some()
                && (listened(H::RecipeSources) || listened(H::PrepareItemCraft)))
    }

    /// Freeze the recipe operation on private storage. Callbacks run only here; apply writes
    /// the kept menu and sources without matching again, asking rules again or retrying.
    pub(super) fn lan_recipe_decided<R>(
        &mut self,
        gi: usize,
        container: i32,
        book: Option<(&str, bool)>,
        publish: bool,
        op: impl FnOnce(&mut Menu, &Env) -> R,
    ) -> Option<R> {
        let conn = self.lan.as_ref()?.hub.guests.get(gi)?.conn;
        let actor = self.guest_actor(conn)?;
        let route = self.guest_route(conn)?;
        let host = self.lan.as_ref()?;
        let generation = host.hub.guests[gi].menu_generation;
        let owners = &host.menus.get(&conn)?.crafting;
        let stamps = (
            owners.inventory.stamp(),
            owners.table.stamp(),
            owners.machine.stamp(),
        );
        let registry = horizon_client::datapack_net::served().map(|r| r.generation());
        let guest = &host.hub.guests[gi];
        let inventory = guest.inventory.clone();
        let grid = guest.craft_grid.clone();
        let cursor = guest.carried.clone();
        let recipes = guest.recipe_book.recipes.clone();
        let creative = guest.creative();
        let limited = host.hub.limited_crafting;
        let planned = Rc::new(RefCell::new(None));
        let result = Rc::new(RefCell::new(None));
        let (keep, take, answer) = (planned.clone(), planned.clone(), result.clone());
        let applied = self.decided_action(
            "recipe placement",
            route,
            actor,
            Epoch::All,
            move |game| {
                let Some(host) = game.lan.as_ref() else {
                    return false;
                };
                let Some(g) = host.hub.guest(conn) else {
                    return false;
                };
                let Some(m) = host.menus.get(&conn) else {
                    return false;
                };
                g.menu_generation == generation
                    && !g.spectator()
                    && !g.dead
                    && g.inventory == inventory
                    && g.craft_grid == grid
                    && g.carried == cursor
                    && g.recipe_book.recipes == recipes
                    && g.creative() == creative
                    && host.hub.limited_crafting == limited
                    && (
                        m.crafting.inventory.stamp(),
                        m.crafting.table.stamp(),
                        m.crafting.machine.stamp(),
                    ) == stamps
                    && horizon_client::datapack_net::served().map(|r| r.generation()) == registry
            },
            move |game| {
                let writes = game.transactions.writes();
                let Some(mut placement) = game.recipe_placement(gi, container) else {
                    return Reply::Failed;
                };
                // Source selection can itself call a listener. Never choose replacement inputs
                // after a listener wrote any of the authoritative storage.
                if game.transactions.writes() != writes {
                    // The coordinator's epoch check supplies Stale before attempting apply.
                    return Reply::Allow;
                }
                let host = game.lan.as_ref().unwrap();
                let guest = &host.hub.guests[gi];
                let env = Env {
                    player: Some(guest.uuid),
                    player_entity_id: Some(guest.entity_id),
                    known_recipes: Some(&guest.recipe_book),
                    limited_crafting: host.hub.limited_crafting,
                    ..game.menu_env()
                };
                let out = op(&mut placement.menu, &env);
                // The selected inputs and every destination are now fixed. The book's notice
                // cannot change the plan, and a Host write makes the entire action stale.
                if let Some((id, all)) = book {
                    let (uuid, name) = (guest.uuid, guest.name.clone());
                    game.native_player_recipe_book_click(uuid, &name, id, all);
                }
                if publish && let Destination::Mod { pos, .. } = placement.destination {
                    let c = placement.menu.mod_crafting().unwrap();
                    let guest = &game.lan.as_ref().unwrap().hub.guests[gi];
                    let (uuid, name) = (guest.uuid, guest.name.clone());
                    let mut output = placement.menu.slot(c.result).cloned();
                    game.native_prepare_item_craft(
                        uuid,
                        &name,
                        Some([pos.0, pos.1, pos.2]),
                        &mut output,
                    );
                    placement.menu.set_slot(&game.menu_env(), c.result, output);
                }
                *keep.borrow_mut() = Some((placement, out));
                Reply::Allow
            },
            move |game| {
                let (placement, out) = take.take().expect("one frozen placement");
                if publish {
                    game.apply_recipe_placement(gi, container, placement);
                }
                *answer.borrow_mut() = Some(out);
            },
        );
        if applied { result.take() } else { None }
    }

    fn recipe_placement(&mut self, gi: usize, container: i32) -> Option<Placement> {
        let conn = self.lan.as_ref()?.hub.guests.get(gi)?.conn;
        let mut sources = Vec::new();
        let (mut menu, destination) =
            if let Some((pos, t)) = self.lan_mod_crafting_window(gi, container) {
                let Some(BlockEntity::Mod(m)) = self.world.block_entities.get(&pos) else {
                    return None;
                };
                let mut items = m.items.clone();
                items.resize(t.slots, None);
                let host = self.lan.as_ref()?;
                let mut menu = host
                    .menus
                    .get(&conn)
                    .and_then(|p| p.crafting.machine.as_ref())
                    .filter(|(w, m)| *w == container && m.kind() == Kind::Mod(t))
                    .map_or_else(|| Box::new(Menu::new(Kind::Mod(t))), |(_, m)| m.clone());
                menu.load_container(&self.menu_env(), &items);
                if let Some((slot, result)) = Self::lan_mod_craft_shown(host, gi, &items) {
                    menu.set_slot(&self.menu_env(), slot, result);
                }
                sources = self.lan_recipe_sources(gi, t, pos);
                menu.sources = sources.iter().map(|(_, items)| items.clone()).collect();
                (menu, Destination::Mod { pos, items })
            } else if container == 0 {
                let host = self.lan.as_ref()?;
                let mut menu = host
                    .menus
                    .get(&conn)
                    .and_then(|p| p.crafting.inventory.as_ref())
                    .cloned()
                    .unwrap_or_else(|| Box::new(Menu::new(Kind::Inventory)));
                for (k, stack) in host.hub.guests[gi].craft_grid.iter().enumerate() {
                    menu.set_slot(&self.menu_env(), k + 1, stack.clone());
                }
                (menu, Destination::Inventory)
            } else if let Some((_, state, menu, pos)) = self
                .lan
                .as_ref()?
                .menus
                .get(&conn)
                .and_then(|p| p.crafting.table.as_ref())
                .filter(|(w, ..)| *w == container)
            {
                (
                    menu.clone(),
                    Destination::Table {
                        state: *state,
                        pos: *pos,
                    },
                )
            } else {
                let host = self.lan.as_ref()?;
                let opened = host.hub.guests[gi]
                    .menu
                    .as_ref()
                    .filter(|m| m.window == container)?;
                let horizon_client::lan::menu::Slots::Block(pos) = opened.slots else {
                    return None;
                };
                let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get(&pos) else {
                    return None;
                };
                let recipe = match f.kind.recipe_type() {
                    "minecraft:blasting" => "blasting",
                    "minecraft:smoking" => "smoking",
                    _ => "smelting",
                };
                let mut menu = Box::new(Menu::new(Kind::Furnace(recipe)));
                for (k, stack) in f.items.iter().enumerate() {
                    menu.set_slot(&self.menu_env(), k, stack.clone());
                }
                (
                    menu,
                    Destination::Furnace {
                        pos,
                        items: f.items.clone(),
                    },
                )
            };
        let guest = &self.lan.as_ref()?.hub.guests[gi];
        menu.set_container_limits(
            guest
                .menu
                .as_ref()
                .map(|m| m.slots.stack_limits(&self.world, m.size))
                .unwrap_or_default(),
        );
        load_player(&mut menu, guest);
        Some(Placement {
            menu,
            destination,
            sources,
        })
    }

    fn apply_recipe_placement(&mut self, gi: usize, container: i32, placement: Placement) {
        let Placement {
            mut menu,
            destination,
            sources,
        } = placement;
        let conn = self.lan.as_ref().unwrap().hub.guests[gi].conn;
        self.journal_guest_click(conn);
        Target::keep_recipe(self, conn);
        let dropped = std::mem::take(&mut menu.dropped);
        menu.drops.clear();
        self.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .update(gi, |g| store_player(&mut menu, g));
        // The order is the order the lower planner consumed sources in. All are journaled
        // before the first write, so even an unwind cannot publish a successful prefix.
        for (pos, _) in &sources {
            self.journal_block_entity(*pos);
        }
        for ((pos, before), after) in sources.into_iter().zip(std::mem::take(&mut menu.sources)) {
            if before != after {
                self.write_recipe_container(pos, after);
            }
        }
        match destination {
            Destination::Inventory => {
                let host = self.lan.as_mut().unwrap();
                host.hub.guests.update(gi, |g| {
                    for k in 0..4 {
                        g.craft_grid[k] = menu.slot(k + 1).cloned();
                    }
                    g.craft_result = menu.slot(0).cloned();
                });
                *host.menus.get_mut(&conn).unwrap().crafting.inventory = Some(menu);
            }
            Destination::Table { state, pos } => {
                *self
                    .lan
                    .as_mut()
                    .unwrap()
                    .menus
                    .get_mut(&conn)
                    .unwrap()
                    .crafting
                    .table = Some((container, state, menu, pos));
            }
            Destination::Mod { pos, items } => {
                let after = menu.container_items();
                if after != items {
                    self.journal_block_entity(pos);
                    self.write_recipe_container(pos, after);
                }
                let c = menu.mod_crafting().unwrap();
                let craft = ModCraft {
                    grid: c.grid_slots().map(|k| menu.slot(k).cloned()).collect(),
                    result: menu.slot(c.result).cloned(),
                };
                let host = self.lan.as_mut().unwrap();
                host.hub.guests.update(gi, |g| {
                    if let Some(m) = g.menu.as_mut() {
                        m.craft = Some(craft);
                    }
                });
                *host.menus.get_mut(&conn).unwrap().crafting.machine = Some((container, menu));
            }
            Destination::Furnace { pos, items } => {
                self.journal_block_entity(pos);
                let changed = (0..3).any(|k| menu.slot(k) != items[k].as_ref());
                if let Some(BlockEntity::Furnace(f)) = self.world.block_entities.get_mut(&pos) {
                    for k in 0..3 {
                        f.items[k] = menu.slot(k).cloned();
                    }
                    f.input_clicked(&self.loop_context, world_recipes!(self), items[0].clone());
                }
                if changed {
                    self.container_changed(pos);
                }
                *self
                    .lan
                    .as_mut()
                    .unwrap()
                    .menus
                    .get_mut(&conn)
                    .unwrap()
                    .crafting
                    .machine = Some((container, menu));
            }
        }
        self.drop_guest_items(gi, dropped);
        let generation = self.lan.as_ref().unwrap().hub.guests[gi].menu_generation;
        self.after_commit(After::Recipe(Publication::Sync {
            conn,
            generation,
            container,
        }));
    }

    pub(super) fn write_recipe_container(&mut self, pos: Pos, after: Vec<Option<ItemStack>>) {
        match self.world.block_entities.get_mut(&pos) {
            Some(BlockEntity::Chest(items)) => *items = after,
            Some(BlockEntity::Mod(m)) => {
                m.items.resize(m.items.len().max(after.len()), None);
                for (slot, stack) in m.items.iter_mut().zip(after) {
                    *slot = stack;
                }
            }
            _ => unreachable!("validated recipe source"),
        }
        self.container_changed(pos);
    }

    pub(super) fn recipe_reply(&mut self, gi: usize, packet: Clientbound) {
        if self.transactions.is_empty() {
            self.lan.as_ref().unwrap().hub.guests[gi].send(&packet);
            return;
        }
        let sender = self.lan.as_ref().unwrap().hub.guests[gi].sender.clone();
        self.after_commit(After::Recipe(Publication::Packet(sender, Box::new(packet))));
    }

    pub(super) fn recipe_ghost(&mut self, gi: usize, container: i32, recipe: &str) {
        let g = &self.lan.as_ref().unwrap().hub.guests[gi];
        self.after_commit(After::Recipe(Publication::Ghost {
            conn: g.conn,
            generation: g.menu_generation,
            container,
            recipe: recipe.into(),
        }));
    }

    pub(in crate::game) fn publish_recipe(&mut self, publication: Publication) {
        if let Publication::Packet(sender, packet) = publication {
            if let Ok(body) = packet.try_encode() {
                sender.send(body);
            }
            return;
        }
        let (conn, generation, container) = match &publication {
            Publication::Sync {
                conn,
                generation,
                container,
            }
            | Publication::Ghost {
                conn,
                generation,
                container,
                ..
            } => (*conn, *generation, *container),
            Publication::Packet(..) => unreachable!(),
        };
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let Some(gi) = host
            .hub
            .guests
            .iter()
            .position(|g| g.conn == conn && g.menu_generation == generation)
        else {
            return;
        };
        match publication {
            Publication::Ghost { recipe, .. } => {
                host.hub.send_ghost_recipe(conn, container, &recipe)
            }
            Publication::Sync { .. } if container == 0 => host.hub.send_inventory(gi),
            Publication::Sync { .. } => {
                self.lan_send_table_content(conn);
            }
            Publication::Packet(..) => unreachable!(),
        }
    }
}
