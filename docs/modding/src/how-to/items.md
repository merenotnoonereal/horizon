# Items, recipes and loot

Pages for mods that work with items and the data around them: recipes and ingredients, enchantment and consume
effects, loot, data maps and conditional data.

## When a player's item is used up

Implement `Hooks::player_destroy_item` to observe the last item consumed by use or by an entity,
a completed food/drink use, or a durability break. The same hook is available to native and wasm mods.
Its `PlayerDestroyItemEvent` contains `player` (UUID), `player_entity_id`, `original` (an immutable full
stack snapshot taken before consumption or wear), and `hand` (`Some(Hand::Main)`, `Some(Hand::Off)`,
or `None` for worn equipment and inventory ammunition). Count, pre-wear damage and component patches
are preserved; a returned bottle or bucket does not replace `original`.

The callback receives `HostSide`: the server sees its applied action, including its local host player,
and a client sees its own prediction, completed use or equipment-break notification. Other players'
inventory synchronization does not produce notices. This is a notification after the change, so it
cannot cancel the action. Consuming only part of a stack and creative use that preserves the item
produce no notice. An enclosing use does not repeat an already reported break or consumption.

See the [API reference](../reference/api.md) for the generated event and `Hooks` method.

<!-- section-pages -->
