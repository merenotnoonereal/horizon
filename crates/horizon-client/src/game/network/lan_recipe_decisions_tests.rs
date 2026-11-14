//! Real guest packets and real listeners, using vanilla's two-stone pressure plate.
use super::{
    Game,
    native_guest_turn_tests::with_guests,
    transactions::decision::{APPLIED, DECIDED},
};
use horizon_client::{
    containers::{BlockEntity, ModBlockEntity, ModBlockEntityFields, ModCrafting, ModMenuType},
    items::ItemStack,
    lan::{
        Out, PlayerSender,
        menu::{Menu as HubMenu, Slots},
        recipes,
    },
};
use horizon_protocol::{
    buf::{Reader, Writer},
    packets::{
        configuration::CustomPayload,
        play::{Clientbound as C, Serverbound as S},
        recipes::PlaceRecipe,
    },
};
use mod_api::{
    Hooks, ItemPlace, ItemResource, ModEntry, PlayerRecipeBookClickEvent, PrepareItemCraftEvent,
    RecipeSourcesEvent, ServerHost,
};
use std::{
    cell::Cell,
    sync::{Arc, LazyLock, mpsc},
};

const RECIPE: &str = "minecraft:stone_pressure_plate";
const POS: (i32, i32, i32) = (1, 64, 1);
const SOURCE: (i32, i32, i32) = (2, 64, 1);
const WINDOW: i32 = 5;

#[derive(Clone, Copy, Default)]
struct Script {
    write: bool,
    panic: bool,
}
thread_local! {
    static SCRIPT: Cell<Script> = const { Cell::new(Script { write: false, panic: false }) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn script(write: bool, panic: bool) {
    SCRIPT.set(Script { write, panic });
    CALLS.set(0);
    DECIDED.set(0);
    APPLIED.set(0);
}

fn listen(host: &mut dyn ServerHost) {
    CALLS.set(CALLS.get() + 1);
    let tx = host
        .transaction_current()
        .expect("callback inside the action");
    let s = SCRIPT.get();
    if s.write {
        let key = mod_api::__private::item_key("minecraft:dirt").unwrap();
        let stack = mod_api::ItemStack::new(key, 1).unwrap();
        let slot = ItemPlace::PlayerSlot {
            player: 12,
            slot: 20,
        };
        assert_eq!(slot.insert(host, &tx, &ItemResource::of(&stack), 2), 2);
    }
    if s.panic {
        panic!("recipe listener failed after its nested write");
    }
}

struct Book;
#[mod_api::__subscriptions]
impl Hooks for Book {
    fn player_recipe_book_click(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerRecipeBookClickEvent<'_>,
    ) {
        assert_eq!(event.recipe, RECIPE);
        listen(host);
    }
}
struct Prepare;
#[mod_api::__subscriptions]
impl Hooks for Prepare {
    fn prepare_item_craft(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PrepareItemCraftEvent<'_>,
    ) -> Option<mod_api::ItemStack> {
        listen(host);
        event.result.clone()
    }
}
struct Sources;
#[mod_api::__subscriptions]
impl Hooks for Sources {
    fn recipe_sources(&self, host: &mut dyn ServerHost, event: &mut RecipeSourcesEvent<'_>) {
        listen(host);
        event.sources.push([SOURCE.0, SOURCE.1, SOURCE.2]);
    }
}

#[derive(Clone, Copy, Debug)]
enum Route {
    Book,
    Mod,
    Transfer,
}

fn setup(game: &mut Game<'_>, route: Route) -> mpsc::Receiver<Out> {
    game.build_registries();
    let (tx, rx) = mpsc::channel();
    game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
        g.loaded = true;
        g.brand = "horizon".into();
        g.sender = PlayerSender::new(tx, Arc::default());
        g.inventory.fill(None);
        g.inventory[0] = Some(ItemStack::new(
            "minecraft:stone",
            if matches!(route, Route::Transfer) {
                1
            } else {
                2
            },
        ));
        g.recipe_book.recipes.insert(RECIPE.into());
    });
    if !matches!(route, Route::Book) {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        static MENU: LazyLock<ModMenuType> = LazyLock::new(|| ModMenuType {
            id: "test:recipes",
            slots: 10,
            data_slots: 0,
            results: vec![],
            links: None,
            crafting: Some(ModCrafting {
                grid: 1,
                size: 3,
                result: 0,
            }),
        });
        game.world.block_entities.insert(
            POS,
            BlockEntity::Mod(Box::new(ModBlockEntity::new(
                "test:recipes".into(),
                ModBlockEntityFields {
                    items: vec![None; 10],
                    ..Default::default()
                },
            ))),
        );
        game.world.block_entities.insert(
            SOURCE,
            BlockEntity::Chest(vec![Some(ItemStack::new("minecraft:stone", 1))]),
        );
        game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
            g.menu = Some(HubMenu {
                window: WINDOW,
                kind: 0,
                registry_id: MENU.id.into(),
                pos: Some(POS),
                slots: Slots::Block(POS),
                size: 10,
                state: 0,
                sent: vec![None; 46],
                drag: None,
                sent_data: vec![],
                hidden: vec![],
                mod_type: Some(&MENU),
                craft: None,
                links: vec![],
                asks_still_valid: false,
            })
        });
    }
    assert!(
        world_recipes!(game).placement(RECIPE).is_some(),
        "prepared vanilla recipe"
    );
    rx
}

fn request(game: &mut Game<'_>, route: Route) {
    let container = if matches!(route, Route::Book) {
        0
    } else {
        WINDOW
    };
    let packet = if matches!(route, Route::Transfer) {
        let mut w = Writer::new();
        w.varint(container);
        w.string(RECIPE);
        w.bool(false);
        S::CustomPayload(CustomPayload {
            channel: recipes::TRANSFER_CHANNEL.into(),
            data: w.data,
        })
    } else {
        let recipe = world_stations!(game)
            .recipe_index
            .first_display(RECIPE)
            .unwrap();
        S::PlaceRecipe(PlaceRecipe {
            container,
            recipe,
            use_max: false,
        })
    };
    game.lan_guest_packet(12, packet);
}

fn state(game: &Game<'_>, route: Route, applied: bool) {
    let g = &game.lan.as_ref().unwrap().hub.guests[0];
    assert_eq!(g.inventory[20], None, "listener's write rolled back");
    assert_eq!(g.carried, None);
    assert_eq!(
        g.inventory[0],
        if applied {
            None
        } else {
            Some(ItemStack::new(
                "minecraft:stone",
                if matches!(route, Route::Transfer) {
                    1
                } else {
                    2
                },
            ))
        }
    );
    let (grid, result) = if matches!(route, Route::Book) {
        (g.craft_grid.to_vec(), g.craft_result.clone())
    } else {
        let Some(BlockEntity::Mod(m)) = game.world.block_entities.get(&POS) else {
            panic!("station remains");
        };
        let result = g
            .menu
            .as_ref()
            .unwrap()
            .craft
            .as_ref()
            .and_then(|c| c.result.clone());
        assert_eq!(m.items[0], None, "result is not stored as an ingredient");
        (m.items[1..10].to_vec(), result)
    };
    assert_eq!(
        grid.into_iter().flatten().collect::<Vec<_>>(),
        if applied {
            vec![ItemStack::new("minecraft:stone", 1); 2]
        } else {
            vec![]
        }
    );
    assert_eq!(
        result,
        applied.then(|| ItemStack::new("minecraft:stone_pressure_plate", 1))
    );
    if matches!(route, Route::Transfer) {
        let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&SOURCE) else {
            panic!("source remains");
        };
        assert_eq!(
            items,
            &vec![if applied {
                None
            } else {
                Some(ItemStack::new("minecraft:stone", 1))
            }]
        );
    }
}

fn packets(rx: &mpsc::Receiver<Out>) -> Vec<C> {
    rx.try_iter()
        .flat_map(|out| match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        })
        .map(|body| {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            C::decode(id, r.rest()).unwrap()
        })
        .collect()
}

fn exercise(route: Route, listener: Box<dyn Hooks>) {
    mod_platform::testing::with_mods(vec![ModEntry::new("recipe_test", listener)].into(), || {
        with_guests(|game| {
            let rx = setup(game, route);
            // Accepted: one dispatch and one application; both stones staged exactly once.
            script(false, false);
            request(game, route);
            assert_eq!((CALLS.get(), DECIDED.get(), APPLIED.get()), (1, 1, 1));
            state(game, route, true);
            let _ = packets(&rx);
        });
        for (write, panic) in [(true, false), (true, true), (false, true)] {
            with_guests(|game| {
                let rx = setup(game, route);
                script(write, panic);
                request(game, route);
                assert_eq!((CALLS.get(), DECIDED.get(), APPLIED.get()), (1, 1, 0));
                state(game, route, false);
                assert!(game.transactions.is_empty());
                let sent = packets(&rx);
                if matches!(route, Route::Transfer) {
                    let reply = sent
                        .iter()
                        .find_map(|p| {
                            if let C::CustomPayload(p) = p {
                                Some(p)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    let mut r = Reader::new(&reply.data);
                    assert_eq!(r.varint().unwrap(), WINDOW);
                    assert_eq!(
                        r.u8().unwrap(),
                        3,
                        "Unavailable is the existing refusal byte"
                    );
                } else {
                    assert!(
                        sent.iter().any(|p| matches!(p, C::PlaceGhostRecipe(_))),
                        "book refusal answers with the ghost"
                    );
                }
                // The failing callback did not strand the guest or the menu; the next input works.
                script(false, false);
                request(game, route);
                assert_eq!((CALLS.get(), APPLIED.get()), (1, 1));
                state(game, route, true);
            });
        }
        with_guests(|game| {
            let rx = setup(game, route);
            script(false, false);
            let outer = game.transaction_open();
            request(game, route);
            state(game, route, true);
            assert_eq!((CALLS.get(), APPLIED.get()), (1, 1));
            assert!(
                packets(&rx).is_empty(),
                "no success packet before root commit"
            );
            game.transaction_close(&outer, false);
            state(game, route, false);
            assert!(packets(&rx).is_empty(), "outer abort publishes no success");
            assert!(game.transactions.is_empty());
        });
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn book_placement_is_one_atomic_decision() {
    exercise(Route::Book, Box::new(Book));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn mod_placement_is_one_atomic_decision() {
    exercise(Route::Mod, Box::new(Prepare));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn transfer_is_one_atomic_decision_with_all_sources() {
    exercise(Route::Transfer, Box::new(Sources));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn unsubscribed_recipe_routes_never_enter_the_coordinator() {
    for route in [Route::Book, Route::Mod, Route::Transfer] {
        with_guests(|game| {
            let rx = setup(game, route);
            // Without a lender the ordinary transfer uses the player's two stones.
            if matches!(route, Route::Transfer) {
                game.lan.as_mut().unwrap().hub.guests.update(0, |g| {
                    g.inventory[0] = Some(ItemStack::new("minecraft:stone", 2))
                });
            }
            script(false, false);
            let writes = game.transactions.writes();
            request(game, route);
            assert_eq!((CALLS.get(), DECIDED.get(), APPLIED.get()), (0, 0, 0));
            assert_eq!(game.transactions.writes(), writes);
            state(
                game,
                if matches!(route, Route::Transfer) {
                    Route::Mod
                } else {
                    route
                },
                true,
            );
            assert!(!packets(&rx).is_empty() || matches!(route, Route::Mod));
            assert!(game.transactions.is_empty());
        });
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_transfer_check_keeps_all_storage_and_uses_the_same_sources_once() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("recipe_test", Box::new(Sources))].into(),
        || {
            with_guests(|game| {
                let rx = setup(game, Route::Transfer);
                let request = |game: &mut Game<'_>| {
                    let mut w = Writer::new();
                    w.varint(WINDOW);
                    w.varint(19);
                    w.string(RECIPE);
                    game.lan_guest_packet(
                        12,
                        S::CustomPayload(CustomPayload {
                            channel: recipes::CHECK_CHANNEL.into(),
                            data: w.data,
                        }),
                    );
                };
                script(false, false);
                request(game);
                assert_eq!((CALLS.get(), DECIDED.get()), (1, 1));
                state(game, Route::Transfer, false);
                let sent = packets(&rx);
                let [C::CustomPayload(reply)] = &sent[..] else {
                    panic!("only the check reply: {sent:?}");
                };
                let mut r = Reader::new(&reply.data);
                assert_eq!(
                    (r.varint().unwrap(), r.varint().unwrap(), r.u8().unwrap()),
                    (WINDOW, 19, 0)
                );
                assert_eq!(r.remaining(), 0);
                script(true, false);
                request(game);
                assert_eq!((CALLS.get(), APPLIED.get()), (1, 0));
                state(game, Route::Transfer, false);
                let sent = packets(&rx);
                let [C::CustomPayload(reply)] = &sent[..] else {
                    panic!("only refusal reply: {sent:?}");
                };
                let mut r = Reader::new(&reply.data);
                assert_eq!(
                    (r.varint().unwrap(), r.varint().unwrap(), r.u8().unwrap()),
                    (WINDOW, 19, 3)
                );
            });
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn unknown_book_recipe_is_dropped_and_missing_items_answer_with_a_ghost() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("recipe_test", Box::new(Book))].into(),
        || {
            with_guests(|game| {
                let rx = setup(game, Route::Book);
                script(false, false);
                game.lan_guest_packet(
                    12,
                    S::PlaceRecipe(PlaceRecipe {
                        container: 0,
                        recipe: -1,
                        use_max: false,
                    }),
                );
                assert_eq!((CALLS.get(), DECIDED.get()), (0, 0));
                assert!(packets(&rx).is_empty());
                state(game, Route::Book, false);
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |g| g.inventory[0] = None);
                request(game, Route::Book);
                assert_eq!((CALLS.get(), APPLIED.get()), (1, 1));
                let g = &game.lan.as_ref().unwrap().hub.guests[0];
                assert!(g.craft_grid.iter().all(Option::is_none));
                assert_eq!(g.craft_result, None);
                assert!(
                    packets(&rx)
                        .iter()
                        .any(|p| matches!(p, C::PlaceGhostRecipe(_)))
                );
            });
        },
    );
}
