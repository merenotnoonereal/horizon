//! Real LAN merchant result takes, with native listeners and the production coordinator.
use super::{Game, native_guest_turn_tests::with_guests};
use horizon_client::{
    items::ItemStack,
    merchant::{Session, Trader},
};
use horizon_protocol::packets::play::ContainerClick;
use mod_api::{Hooks, ItemPlace, ItemResource, ModEntry, Mods, ServerHost, TradeWithVillagerEvent};
use std::cell::{Cell, RefCell};

#[derive(Clone, Default)]
struct Script {
    panic_on: Option<usize>,
    answer: bool,
    invalid: bool,
}
thread_local! {
    static SCRIPT: RefCell<Script> = RefCell::new(Script::default());
    static CALLS: Cell<usize> = const { Cell::new(0) };
    static IN_ACTION: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}
fn script(value: Script) {
    SCRIPT.with(|s| *s.borrow_mut() = value);
    CALLS.set(0);
    IN_ACTION.with(|a| a.borrow_mut().clear());
    super::decided_action::DECIDED.set(0);
}
struct Mod;
#[mod_api::__subscriptions]
impl Hooks for Mod {
    fn trade_with_villager(
        &self,
        host: &mut dyn ServerHost,
        event: &mut TradeWithVillagerEvent<'_>,
    ) {
        let call = CALLS.get() + 1;
        CALLS.set(call);
        IN_ACTION.with(|a| a.borrow_mut().push(host.transaction_current().is_some()));
        assert_eq!(event.merchant, 40);
        let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
        let dirt = mod_api::ItemStack::new(key, 1).unwrap();
        let place = ItemPlace::PlayerSlot {
            player: 12,
            slot: 20,
        };
        let tx = host
            .transaction_current()
            .expect("trade listener is inside the action");
        assert_eq!(place.insert(host, &tx, &ItemResource::of(&dirt), 2), 2);
        let s = SCRIPT.with(|s| s.borrow().clone());
        if s.answer {
            event.offer.uses = 6;
            event.offer.max_uses = 19;
            event.offer.demand = -7;
            event.offer.special_price_diff = -4;
            event.offer.price_multiplier = 0.25;
            event.offer.xp = 11;
        }
        if s.invalid {
            event.offer.price_multiplier = f32::NAN;
        }
        if s.panic_on == Some(call) {
            panic!("trade listener failed");
        }
    }
}
fn with_mod(test: impl FnOnce()) {
    let mods: Mods = vec![ModEntry::new("trade-observer", Box::new(Mod))].into();
    mod_platform::testing::with_mods(mods, test);
}

fn open(game: &mut Game<'_>, payment: u32) -> ContainerClick {
    use super::{
        native_merchant_refs::tests::{offer, spawn},
        trading::MerchantState,
    };
    spawn(game, 40, "minecraft:villager");
    game.mobs[0].extra.profession = horizon_client::mobs::villager::ProfessionState::from_index(5);
    let mut offer = offer("minecraft:wheat", "minecraft:emerald");
    offer.base_cost_a.stack.count = 1;
    offer.uses = 0;
    offer.max_uses = 10;
    offer.demand = 0;
    offer.special_price_diff = 0;
    offer.price_multiplier = 0.0;
    game.mobs[0].extra.trade = Some(Box::new(Trader {
        offers: vec![offer.clone()],
        ..Trader::default()
    }));
    game.mobs[0].extra.flag2 = true;
    let mut session = Session::new(vec![offer], true);
    // Avoid a level-up changing the one-offer test while still awarding real trade XP.
    session.level = 5;
    session.profession = "minecraft:farmer".to_owned();
    game.lan_open_merchant(
        12,
        MerchantState {
            session: Box::new(session),
            scroll: 0,
            dragging: false,
            focused: None,
            mob: Some(40),
            name: None,
        },
    );
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.inventory[9] = Some(ItemStack::new("minecraft:wheat", payment));
    });
    game.lan_select_trade(12, 0);
    let window = game.lan.as_ref().unwrap().play[&12]
        .merchant
        .as_ref()
        .unwrap()
        .window;
    ContainerClick {
        container: window,
        state: 0,
        slot: 2,
        button: 0,
        input: 0,
        changed: Vec::new(),
        carried: None,
    }
}
fn count(stack: Option<&ItemStack>) -> u32 {
    stack.map_or(0, |s| s.count)
}
fn result(game: &Game<'_>) -> u32 {
    let guest = &game.lan.as_ref().unwrap().hub.guests[0];
    guest
        .inventory
        .iter()
        .flatten()
        .filter(|s| s.item.as_ref() == "minecraft:emerald")
        .map(|s| s.count)
        .sum::<u32>()
        + count(
            guest
                .carried
                .as_ref()
                .filter(|s| s.item.as_ref() == "minecraft:emerald"),
        )
}
fn dirt(game: &Game<'_>) -> u32 {
    count(game.lan.as_ref().unwrap().hub.guests[0].inventory[20].as_ref())
}
fn stat(game: &Game<'_>) -> i32 {
    game.lan.as_ref().unwrap().play[&12]
        .stats
        .values
        .get("minecraft:custom")
        .and_then(|v| v.get("minecraft:traded_with_villager"))
        .copied()
        .unwrap_or(0)
}
fn unchanged(game: &mut Game<'_>, payment: u32) {
    assert_eq!(result(game), 0);
    assert_eq!(dirt(game), 0);
    assert_eq!(stat(game), 0);
    let session = game.lan_trade_session(12).unwrap();
    assert_eq!(session.offers[0].uses, 0);
    assert_eq!(session.xp, 0);
    assert!(game.orbs.is_empty());
    let host = game.lan.as_ref().unwrap();
    let slots = &host.play[&12].merchant.as_ref().unwrap().slots;
    assert_eq!(count(slots[0].as_ref()), payment);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn accepted_trade_keeps_the_collect_answer_and_listener_writes() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                answer: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            assert_eq!(super::decided_action::DECIDED.get(), 1);
            assert_eq!(IN_ACTION.with(|a| a.borrow().clone()), [true]);
            assert_eq!(result(game), 1);
            assert_eq!(dirt(game), 2);
            assert_eq!(stat(game), 1);
            let session = game.lan_trade_session(12).unwrap();
            assert_eq!(session.xp, 7);
            let offer = &session.offers[0];
            assert_eq!(
                (
                    offer.uses,
                    offer.max_uses,
                    offer.demand,
                    offer.special_price_diff,
                    offer.xp
                ),
                (6, 19, -7, -4, 11)
            );
            assert_eq!(offer.price_multiplier, 0.25);
            assert!(!game.orbs.is_empty());
            assert!(game.transactions.is_empty());
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_panicking_trade_listener_undoes_the_take_and_the_next_trade_works() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                panic_on: Some(1),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((CALLS.get(), result(game), stat(game)), (1, 1, 1));
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_second_trade_panic_undoes_the_entire_shift_click() {
    with_mod(|| {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script {
                panic_on: Some(2),
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 2);
            assert_eq!(super::decided_action::DECIDED.get(), 1);
            unchanged(game, 3);
            assert!(game.transactions.is_empty());
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((CALLS.get(), result(game), stat(game)), (3, 3, 3));
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_outer_abort_undoes_an_accepted_trade() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script::default());
            let seal = game.transaction_open();
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!((result(game), dirt(game), stat(game)), (1, 2, 1));
            assert!(game.orbs.is_empty(), "orb creation waits for root commit");
            game.transaction_close(&seal, false);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_non_finite_collect_answer_refuses_the_whole_take() {
    with_mod(|| {
        with_guests(|game| {
            let click = open(game, 2);
            script(Script {
                invalid: true,
                ..Script::default()
            });
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(CALLS.get(), 1);
            unchanged(game, 2);
        })
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn no_subscriber_uses_the_ordinary_trade_without_a_decision() {
    mod_platform::testing::with_mods(Mods::default(), || {
        with_guests(|game| {
            let mut click = open(game, 3);
            click.input = 1;
            script(Script::default());
            assert!(game.lan_merchant_click(12, &click));
            assert_eq!(super::decided_action::DECIDED.get(), 0);
            assert_eq!(CALLS.get(), 0);
            assert_eq!((result(game), stat(game)), (3, 3));
            assert_eq!(game.lan_trade_session(12).unwrap().offers[0].uses, 3);
            assert_eq!(game.lan_trade_session(12).unwrap().xp, 21);
            assert!(game.transactions.is_empty());
        })
    });
}
