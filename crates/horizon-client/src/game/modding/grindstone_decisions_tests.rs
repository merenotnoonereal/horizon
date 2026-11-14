//! `grindstone_place` and `grindstone_take` through real listeners: both run as decided actions
//! on a guest's workstation menu, with the same savepoint and refusal as the other menu hooks.
//! The fixtures need prepared resources.
use crate::game::{
    native_guest_turn_tests::with_guests,
    stations::{Kind as StationKind, Station},
};
use horizon_client::items::ItemStack;
use horizon_protocol::packets::play::{ContainerClick, Serverbound};
use mod_api::{
    Flow, GrindstonePlaceEvent, GrindstoneTakeEvent, Hooks, ItemPlace, ItemResource, ModEntry, Mods,
    Registrar, ServerHost,
};
use std::cell::{Cell, RefCell};

#[derive(Clone, Default)]
struct Script {
    place_output: bool,
    place_cancel: bool,
    place_write: bool,
    place_bad_xp: bool,
    take_panic: bool,
}

thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static DISPATCHES: Cell<usize> = const { Cell::new(0) };
    static TAKE_XP: Cell<Option<i32>> = const { Cell::new(None) };
}

fn script(script: Script) {
    DISPATCHES.set(0);
    TAKE_XP.set(None);
    SCRIPT.with(|s| *s.borrow_mut() = script);
}

fn dirt(count: u32) -> mod_api::ItemStack {
    mod_api::ItemStack::new(
        mod_api::__private::item_key("minecraft:dirt").unwrap(),
        count,
    )
    .unwrap()
}

fn write_dirt(host: &mut dyn ServerHost) {
    let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
    let dirt = mod_api::ItemStack::new(key, 1).unwrap();
    let place = ItemPlace::PlayerSlot {
        player: 12,
        slot: 20,
    };
    let tx = host.transaction_current().expect("inside the action");
    assert_eq!(place.insert(host, &tx, &ItemResource::of(&dirt), 2), 2);
}

struct Mod;

#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn register(&self, _: &mut dyn Registrar, _: &mod_api::RegistrationContext) {}

    fn grindstone_place(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstonePlaceEvent,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        assert!(host.transaction_current().is_some());
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.place_write {
            write_dirt(host);
        }
        if s.place_output {
            event.place.output = Some(dirt(1));
            event.place.xp = 5;
        }
        if s.place_bad_xp {
            event.place.xp = -1;
        }
        if s.place_cancel {
            event.cancelled = true;
        }
        Flow::Continue
    }

    fn grindstone_take(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GrindstoneTakeEvent,
    ) -> mod_api::Flow {
        DISPATCHES.set(DISPATCHES.get() + 1);
        TAKE_XP.set(Some(event.take.xp));
        assert!(host.transaction_current().is_some());
        if SCRIPT.with(|s| s.borrow().take_panic) {
            panic!("a listener failed");
        }
        Flow::Continue
    }
}

fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("grindmod", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, || {
        assert!(mod_api::__private::listened(mod_api::HookId::GrindstonePlace));
        assert!(mod_api::__private::listened(mod_api::HookId::GrindstoneTake));
        test();
    });
}

fn without_mod(test: impl FnOnce()) {
    let mods: Mods = Vec::new().into();
    mod_platform::testing::with_mods(mods, test);
}

fn engine_item(game: &mut crate::game::Game<'_>, id: &str, count: u32) -> ItemStack {
    let key = mod_api::__private::item_key(id).unwrap();
    {
        use crate::game::api_values::*;
        mod_api::ItemStack::new(key, count)
            .unwrap()
            .try_into_engine(game.items())
    }
    .ok()
    .unwrap()
}

fn open_grindstone(game: &mut crate::game::Game<'_>) -> i32 {
    // ContainerClick checks stillValid before routing to the workstation. Give the menu a
    // real block, as the other workstation fixtures do, so that check keeps it open.
    game.world.chunks.insert(
        (0, 0),
        std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
    );
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:grindstone")
        .unwrap();
    game.world.set(1, 64, 1, state).unwrap();
    assert!(game.lan_open_station(12, &Station::new(StationKind::Grindstone, [1, 64, 1])));
    let host = game.lan.as_ref().unwrap();
    assert!(game.lan_station_valid(
        &host.hub.guests[0],
        host.menus[&12].station.as_ref().unwrap(),
    ));
    game.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .window
}

fn enchanted_sword(game: &mut crate::game::Game<'_>, damage: u32) -> ItemStack {
    let mut sword = engine_item(game, "minecraft:iron_sword", 1);
    sword.damage = damage;
    sword.set_crafting_enchantments(vec![("minecraft:sharpness".into(), 3)]);
    sword
}

fn slot(game: &crate::game::Game<'_>, index: usize) -> Option<ItemStack> {
    game.lan.as_ref().unwrap().menus[&12]
        .station
        .as_ref()
        .unwrap()
        .menu
        .slot(index)
        .cloned()
}

fn count(stack: &Option<ItemStack>) -> u32 {
    stack.as_ref().map_or(0, |s| s.count)
}

fn inventory(game: &crate::game::Game<'_>, index: usize) -> u32 {
    count(&game.lan.as_ref().unwrap().hub.guests[0].inventory[index])
}

fn container_click(window: i32, slot: i16, button: i8) -> Serverbound {
    Serverbound::ContainerClick(ContainerClick {
        state: 0,
        changed: Vec::new(),
        carried: None,
        container: window,
        slot,
        button,
        input: 0,
    })
}

fn click(game: &mut crate::game::Game<'_>, window: i32, slot: i16, button: i8) {
    game.lan_guest_packet(12, container_click(window, slot, button));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_place_listener_sets_the_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            click(game, window, 2, 0);
            assert_eq!(TAKE_XP.get(), Some(5));
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].carried,
                Some(ItemStack::new("minecraft:dirt", 1))
            );
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_cancelled_place_leaves_no_grindstone_result() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_cancel: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_stale_place_is_refused_and_the_listener_write_undone() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_output: true,
                place_write: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
            assert_eq!(inventory(game, 20), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_invalid_place_answer_is_refused() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            script(Script {
                place_bad_xp: true,
                ..Script::default()
            });
            click(game, window, 0, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(count(&slot(game, 2)), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_panicking_take_listener_leaves_the_grindstone_as_it_was() {
    with_mod(|| {
        with_guests(|game| {
            let window = open_grindstone(game);
            let sword = enchanted_sword(game, 200);
            let original = sword.clone();
            script(Script {
                place_output: true,
                ..Script::default()
            });
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(sword);
            });
            click(game, window, 0, 0);
            script(Script {
                take_panic: true,
                ..Script::default()
            });
            click(game, window, 2, 0);
            assert_eq!(DISPATCHES.get(), 1);
            assert_eq!(slot(game, 2), Some(ItemStack::new("minecraft:dirt", 1)));
            assert_eq!(slot(game, 0), Some(original));
            assert_eq!(slot(game, 1), None);
            let carried = game.lan.as_ref().unwrap().hub.guests[0].carried.clone();
            assert_eq!(count(&carried), 0);
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_grindstone_without_a_listener_keeps_the_vanilla_result() {
    without_mod(|| {
        with_guests(|game| {
            assert!(!mod_api::__private::listened(mod_api::HookId::GrindstonePlace));
            assert!(!mod_api::__private::listened(mod_api::HookId::GrindstoneTake));
            let window = open_grindstone(game);
            let a = enchanted_sword(game, 200);
            let b = enchanted_sword(game, 150);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(a);
            });
            click(game, window, 0, 0);
            game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                g.carried = Some(b);
            });
            click(game, window, 1, 0);
            assert!(
                game.lan
                    .as_ref()
                    .unwrap()
                    .menus
                    .get(&12)
                    .is_some_and(|p| p.station.is_some()),
                "the grindstone station is gone after the click"
            );
            // Iron swords have 250 durability: 50 + 100 remaining + floor(250 * .05)
            // leaves 162 durability, i.e. damage 88. Sharpness is removed.
            let mut expected = ItemStack::new("minecraft:iron_sword", 1);
            expected.damage = 88;
            assert_eq!(slot(game, 2), Some(expected));
        });
    });
}
