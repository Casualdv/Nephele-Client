//! Client-side half of container clicks.
//!
//! Slots are server-authoritative, but the stack under the cursor only exists on
//! the client, and single-player has no server at all. These helpers keep both
//! consistent between a click and the next state sync. They mirror the rules the
//! mock enforces: left click takes all / places all / merges / swaps, right
//! click takes half / places one.

use nephele_protocol::packets::play::ItemStack;

const MAX_STACK: i8 = 64;

fn same(a: ItemStack, b: ItemStack) -> bool {
    a.id == b.id && a.damage == b.damage
}

/// Apply one normal click to `(cursor, slot)` and return what they become.
pub fn click(
    cursor: Option<ItemStack>,
    slot: Option<ItemStack>,
    button: i8,
) -> (Option<ItemStack>, Option<ItemStack>) {
    match (cursor, slot, button) {
        // Left click.
        (None, Some(on_slot), 0) => (Some(on_slot), None),
        (Some(held), None, 0) => (None, Some(held)),
        (Some(held), Some(on_slot), 0) if same(held, on_slot) && on_slot.count < MAX_STACK => {
            let take = held.count.min(MAX_STACK - on_slot.count);
            let merged = ItemStack::new(on_slot.id, on_slot.count + take, on_slot.damage);
            let left = held.count - take;
            let cursor = if left == 0 {
                None
            } else {
                Some(ItemStack::new(held.id, left, held.damage))
            };
            (cursor, Some(merged))
        }
        (Some(held), Some(on_slot), 0) => (Some(on_slot), Some(held)),

        // Right click.
        (None, Some(on_slot), _) if on_slot.count > 0 => {
            let take = (on_slot.count + 1) / 2;
            let left = on_slot.count - take;
            let slot = if left == 0 {
                None
            } else {
                Some(ItemStack::new(on_slot.id, left, on_slot.damage))
            };
            (Some(ItemStack::new(on_slot.id, take, on_slot.damage)), slot)
        }
        (Some(held), None, _) => {
            let left = held.count - 1;
            let cursor = if left == 0 {
                None
            } else {
                Some(ItemStack::new(held.id, left, held.damage))
            };
            (cursor, Some(ItemStack::new(held.id, 1, held.damage)))
        }
        (Some(held), Some(on_slot), _) if same(held, on_slot) && on_slot.count < MAX_STACK => {
            let left = held.count - 1;
            let cursor = if left == 0 {
                None
            } else {
                Some(ItemStack::new(held.id, left, held.damage))
            };
            (
                cursor,
                Some(ItemStack::new(
                    on_slot.id,
                    on_slot.count + 1,
                    on_slot.damage,
                )),
            )
        }
        (cursor, slot, _) => (cursor, slot),
    }
}

/// Shift-click: move `slot` into the other region (hotbar <-> storage),
/// merging into matching stacks first. Returns the slots that changed.
///
/// Only used in single-player; against a server the server performs the move.
pub fn quick_move(inventory: &mut [Option<ItemStack>], slot: usize) -> Vec<usize> {
    const STORAGE: std::ops::Range<usize> = 9..36;
    const HOTBAR: std::ops::Range<usize> = 36..45;

    let mut changed = Vec::new();
    let Some(mut stack) = inventory.get(slot).copied().flatten() else {
        return changed;
    };
    let original = stack.count;
    let targets = if HOTBAR.contains(&slot) {
        STORAGE
    } else {
        HOTBAR
    };

    for target in targets {
        match inventory.get(target).copied().flatten() {
            None => {
                inventory[target] = Some(stack);
                inventory[slot] = None;
                changed.extend([slot, target]);
                return changed;
            }
            Some(on_slot) if same(on_slot, stack) && on_slot.count < MAX_STACK => {
                let take = stack.count.min(MAX_STACK - on_slot.count);
                inventory[target] = Some(ItemStack::new(
                    on_slot.id,
                    on_slot.count + take,
                    on_slot.damage,
                ));
                stack.count -= take;
                if !changed.contains(&target) {
                    changed.push(target);
                }
                if stack.count == 0 {
                    inventory[slot] = None;
                    changed.push(slot);
                    return changed;
                }
            }
            _ => {}
        }
    }

    if stack.count != original {
        inventory[slot] = Some(stack);
        changed.push(slot);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: i16, count: i8) -> Option<ItemStack> {
        Some(ItemStack::new(id, count, 0))
    }

    #[test]
    fn left_click_takes_places_and_swaps() {
        let log = stack(17, 3).unwrap();
        let (cursor, slot) = click(None, Some(log), 0);
        assert_eq!(cursor, Some(log));
        assert_eq!(slot, None);

        let (cursor, slot) = click(Some(log), None, 0);
        assert_eq!(cursor, None);
        assert_eq!(slot, Some(log));

        let (cursor, slot) = click(stack(1, 2), stack(3, 5), 0);
        assert_eq!(cursor, stack(3, 5));
        assert_eq!(slot, stack(1, 2));
    }

    #[test]
    fn left_click_merges_up_to_a_full_stack() {
        let (cursor, slot) = click(stack(1, 60), stack(1, 10), 0);
        assert_eq!(cursor, stack(1, 6));
        assert_eq!(slot, stack(1, 64));

        let (cursor, slot) = click(stack(1, 2), stack(1, 62), 0);
        assert_eq!(cursor, None);
        assert_eq!(slot, stack(1, 64));

        let (cursor, slot) = click(stack(1, 4), stack(1, 62), 0);
        assert_eq!(cursor, stack(1, 2), "the overflow stays on the cursor");
        assert_eq!(slot, stack(1, 64));
    }

    #[test]
    fn right_click_takes_half_and_places_one() {
        let (cursor, slot) = click(None, stack(5, 5), 1);
        assert_eq!(cursor, stack(5, 3));
        assert_eq!(slot, stack(5, 2));

        let (cursor, slot) = click(stack(5, 3), stack(5, 1), 1);
        assert_eq!(cursor, stack(5, 2));
        assert_eq!(slot, stack(5, 2));
    }

    #[test]
    fn quick_move_crosses_regions_and_merges() {
        let mut inventory = vec![None; 45];
        inventory[36] = stack(17, 2);
        let changed = quick_move(&mut inventory, 36);
        assert_eq!(inventory[36], None);
        assert_eq!(inventory[9], stack(17, 2));
        assert_eq!(changed, vec![36, 9]);

        // The same item already in storage merges instead of using a new slot.
        inventory[9] = stack(17, 63);
        inventory[36] = stack(17, 2);
        quick_move(&mut inventory, 36);
        assert_eq!(inventory[9], stack(17, 64));
        assert_eq!(
            inventory[10],
            stack(17, 1),
            "the rest spills to the next slot"
        );
        assert_eq!(inventory[36], None);
    }
}
