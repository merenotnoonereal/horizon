pub(super) struct ItemStackedOnOtherHooks;
impl ItemStackedOnOtherHooks {
    pub(super) fn item_stacked_on_other(
        &self,
        _: mod_api::HostSide,
        event: &mut mod_api::ItemStackedOnOtherEvent<'_>,
    ) -> mod_api::Flow {
        // A cancelled, handled click that moves the cursor's stack into the slot.
        use mod_api::item::menu::StackResult;
        event.result.handled = true;
        event.result.clicked = StackResult::Set(event.event.carried.clone());
        event.result.carried = StackResult::Set(None);
        event.cancelled = true;
        mod_api::Flow::Continue
    }
}
