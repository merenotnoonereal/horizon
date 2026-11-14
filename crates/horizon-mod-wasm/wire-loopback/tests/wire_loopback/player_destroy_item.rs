use super::*;

thread_local! {
    static HEARD: std::cell::RefCell<Vec<(bool, u128, i32, Option<mod_api::Hand>, mod_api::ItemStack)>> = const { std::cell::RefCell::new(Vec::new()) };
}

pub(super) struct PlayerDestroyItemHooks;
impl PlayerDestroyItemHooks {
    pub(super) fn player_destroy_item(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::PlayerDestroyItemEvent<'_>,
    ) {
        HEARD.with_borrow_mut(|heard| {
            heard.push((
                matches!(host, mod_api::HostSide::Client(_)),
                event.player,
                event.player_entity_id,
                event.hand,
                event.original.clone(),
            ))
        });
    }
}

#[test]
fn original_destroyed_stack_and_optional_hand_cross_the_guest_boundary() {
    struct World;
    impl mod_api::Host for World {}
    mod_api::impl_host_areas!(World);
    impl mod_api::ServerHost for World {}
    mod_api::impl_server_host_areas!(World);
    impl mod_api::ClientHost for World {}
    mod_api::impl_client_host_areas!(World);
    install_counter();
    let mut original = mod_api::ItemStack::new(REGISTERED_ITEM.get().unwrap(), 7).unwrap();
    original.set_damage(37);
    let mut data = mod_api::Compound::new();
    data.insert("marker".into(), mod_api::Tag::Long(0x1234_5678_9abc_def0));
    let mut patch = mod_api::Compound::new();
    patch.insert("minecraft:custom_data".into(), mod_api::Tag::Compound(data));
    original.set_components(patch).unwrap();
    let player = 0xfedc_ba98_7654_3210_0123_4567_89ab_cdef;
    for client in [false, true] {
        for hand in [Some(mod_api::Hand::Main), Some(mod_api::Hand::Off), None] {
            let mut world = World;
            WasmMod {
                subscribed: &[],
                listeners: &[],
                group: 0,
                corrupt: false,
            }
            .player_destroy_item(
                if client {
                    mod_api::HostSide::Client(&mut world)
                } else {
                    mod_api::HostSide::Server(&mut world)
                },
                &mut mod_api::PlayerDestroyItemEvent {
                    player,
                    player_entity_id: 73,
                    original: &original,
                    hand,
                },
            );
            HEARD.with_borrow_mut(|heard| {
                assert_eq!(
                    heard.as_slice(),
                    &[(client, player, 73, hand, original.clone())]
                );
                heard.clear();
            });
        }
    }
}
