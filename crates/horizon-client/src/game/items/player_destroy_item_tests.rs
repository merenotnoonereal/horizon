//! Full stack and hand proofs at real consumption and durability write paths.
use super::*;
use horizon_protocol::connection::Pipe;
use mod_api::{Hooks, HostSide, PlayerDestroyItemEvent};
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Notice {
    client: bool,
    player: u128,
    entity: i32,
    hand: Option<Hand>,
    original: mod_api::ItemStack,
}

#[derive(Clone, Default)]
struct Observer(Arc<Mutex<Vec<Notice>>>);

#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn player_destroy_item(&self, host: HostSide<'_>, event: &mut PlayerDestroyItemEvent<'_>) {
        self.0.lock().unwrap().push(Notice {
            client: matches!(host, HostSide::Client(_)),
            player: event.player,
            entity: event.player_entity_id,
            hand: event.hand,
            original: event.original.clone(),
        });
    }
}

fn named_stack(item: &str, count: u32) -> ItemStack {
    let mut stack = ItemStack::new(item, count);
    stack.data_mut().set_loot_name(
        "minecraft:custom_name",
        &serde_json::json!("Destruction snapshot"),
    );
    stack
}

fn take_notice(observer: &Observer, hand: Option<Hand>, item: &str, damage: u32) -> Notice {
    let mut notices = observer.0.lock().unwrap();
    assert_eq!(notices.len(), 1, "each exhausted stack is heard once");
    let notice = notices.pop().unwrap();
    assert_eq!(notice.hand, hand);
    assert_eq!(notice.original.count(), 1);
    assert_eq!(notice.original.damage(), damage);
    assert_eq!(
        mod_api::__private::item_key_id(notice.original.item()).unwrap(),
        item
    );
    assert!(notice.original.components().is_some());
    notice
}

#[test]
#[ignore = "daily"]
fn original_stack_and_hand_reach_used_up_and_broken_item_notices() {
    in_own_process(
        "game::player_destroy_item_tests::original_stack_and_hand_reach_used_up_and_broken_item_notices",
        run_isolated,
    );
}

fn run_isolated() {
    let observer = Observer::default();
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "destroy_observer",
        Box::new(observer.clone()),
    )]
    .into();
    mod_platform::testing::with_mods(mods.clone(), || {
        crate::game::api_values::install_parts(&mods).unwrap();
        for client in [false, true] {
            native_entity_refs::tests::with_game(|game| {
                let _peer = if client {
                    game.options.execution_role = execution_role::ExecutionRole::Client;
                    let (local, peer) = Pipe::pair();
                    game.connect_local(local, "DestroyTest").unwrap();
                    Some(peer)
                } else {
                    None
                };
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
                game.player.as_mut().unwrap().entity_id = 71;
                game.set_mode_id(0);
                game.selected = 0;
                game.level.player_uuid = Some([1, 2, 3, 4]);
                game.screen = Screen::Playing;
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
                // Both hands, partial consumption, and creative preservation.
                for hand in Hand::BOTH {
                    game.with_hand(hand, |game| {
                        *game.held_mut() = Some(named_stack("minecraft:apple", 2));
                        game.consume_held();
                        assert!(observer.0.lock().unwrap().is_empty());
                        game.creative = true;
                        game.consume_held();
                        assert_eq!(game.held().unwrap().count, 1);
                        assert!(observer.0.lock().unwrap().is_empty());
                        game.creative = false;
                        game.consume_held();
                        assert!(game.held().is_none());
                        let notice = take_notice(&observer, Some(hand), "minecraft:apple", 0);
                        assert_eq!(notice.client, client);
                        assert_eq!(notice.entity, if client { 0 } else { 71 });
                        if !client {
                            assert_eq!(notice.player, 0x0000_0001_0000_0002_0000_0003_0000_0004);
                        }
                    });
                }
                // Actual Item.use: a knowledge book is exhausted even when it has no recipes.
                game.offhand = Some(named_stack("minecraft:knowledge_book", 1));
                if client {
                    game.remote_use_item(Hand::Off);
                } else {
                    game.with_hand(Hand::Off, |g| g.use_hand(None)).unwrap();
                }
                take_notice(&observer, Some(Hand::Off), "minecraft:knowledge_book", 0);

                // Completed use must report the consumed potion, not its glass bottle remainder.
                let potion = named_stack("minecraft:potion", 1);
                game.offhand = Some(potion.clone());
                let info = game.items().get(&potion.item).unwrap().clone();
                if client {
                    game.remote_use_item_finished(shield::OFFHAND_SLOT, &potion, &info);
                } else {
                    game.with_hand(Hand::Off, |g| {
                        g.complete_using_item(shield::OFFHAND_SLOT, &potion, &info)
                    });
                }
                assert_eq!(
                    &*game.offhand.as_ref().unwrap().item,
                    "minecraft:glass_bottle"
                );
                take_notice(&observer, Some(Hand::Off), "minecraft:potion", 0);

                if !client {
                    // The same notice identifies a swapped-in guest rather than the local host.
                    game.lan_acting_uuid = Some(0x1111_2222_3333_4444_5555_6666_7777_8888);
                    game.lan_acting_entity = Some(123);
                    game.offhand = Some(named_stack("minecraft:apple", 1));
                    game.with_hand(Hand::Off, Game::consume_held);
                    let guest = take_notice(&observer, Some(Hand::Off), "minecraft:apple", 0);
                    assert_eq!(guest.player, 0x1111_2222_3333_4444_5555_6666_7777_8888);
                    assert_eq!(guest.entity, 123);
                    game.lan_acting_uuid = None;
                    game.lan_acting_entity = None;

                    // Native entity interaction: the last wheat is eaten by an adult cow.
                    let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
                    game.mobs.clear();
                    game.mobs.push(horizon_client::mobs::Mob::new(
                        42,
                        kind,
                        Vec3::new(0.5, 64.0, 2.5),
                        0.0,
                    ));
                    game.offhand = Some(named_stack("minecraft:wheat", 1));
                    assert!(
                        game.with_hand(Hand::Off, |g| g.interact_entity(Some(InteractTarget {
                            id: 42,
                            location: [0.0, 1.0, 0.0],
                        })))
                        .unwrap()
                    );
                    assert!(game.offhand.is_none());
                    take_notice(&observer, Some(Hand::Off), "minecraft:wheat", 0);

                    // Durability reports pre-wear damage, and no duplicate from the enclosing use.
                    let mut tool = named_stack("minecraft:iron_pickaxe", 1);
                    tool.damage = 249;
                    game.offhand = Some(tool);
                    game.with_hand(Hand::Off, |g| {
                        g.with_item_destroy_notice(Hand::Off, |g| g.damage_held(1))
                    });
                    assert!(game.offhand.is_none());
                    take_notice(&observer, Some(Hand::Off), "minecraft:iron_pickaxe", 249);

                    // Armour has no hand; handle-driven wear must keep the same original snapshot.
                    let mut helmet = named_stack("minecraft:iron_helmet", 1);
                    helmet.damage = 164;
                    game.armor[0] = Some(helmet);
                    game.damage_player_equipment(mod_api::EquipmentSlot::Head, 1);
                    assert!(game.armor[0].is_none());
                    take_notice(&observer, None, "minecraft:iron_helmet", 164);
                }
                assert!(observer.0.lock().unwrap().is_empty());
            });
        }
    });
}
