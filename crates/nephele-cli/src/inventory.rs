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

/// Match the shared recipe book against a 3x3 grid.
pub fn crafting_result(grid: &[Option<ItemStack>; 9]) -> Option<ItemStack> {
    if grid
        .iter()
        .flatten()
        .any(|item| item.id < 0 || item.count <= 0)
    {
        return None;
    }
    let ingredients = std::array::from_fn(|index| {
        grid[index].map(|item| (item.id as u16, item.damage.max(0) as u16))
    });
    let recipe = nephele_world::crafting::craft(&ingredients)?;
    Some(ItemStack::new(
        recipe.id as i16,
        recipe.count as i8,
        recipe.damage as i16,
    ))
}

/// Match the 2x2 crafting grid in the player inventory (slots 1..=4).
pub fn player_crafting_result(slots: &[Option<ItemStack>]) -> Option<ItemStack> {
    if slots.len() < 5 {
        return None;
    }
    let mut grid = [None; 9];
    for (grid_slot, inventory_slot) in [(0, 1), (1, 2), (3, 3), (4, 4)] {
        grid[grid_slot] = slots[inventory_slot];
    }
    crafting_result(&grid)
}

/// Match the 3x3 crafting grid in an open crafting-table window (slots 1..=9).
pub fn table_crafting_result(slots: &[Option<ItemStack>]) -> Option<ItemStack> {
    if slots.len() < 10 {
        return None;
    }
    let grid = std::array::from_fn(|index| slots[index + 1]);
    crafting_result(&grid)
}

/// Consume one item from each occupied crafting cell.
pub fn consume_crafting_grid(grid: &mut [Option<ItemStack>]) {
    for slot in grid {
        let Some(mut item) = *slot else {
            continue;
        };
        item.count -= 1;
        *slot = (item.count > 0).then_some(item);
    }
}

/// Put a complete crafting result on the cursor, or merge it if the whole
/// output fits. Returns false without changing the cursor when it cannot fit.
pub fn take_crafting_result(cursor: &mut Option<ItemStack>, result: ItemStack) -> bool {
    if result.count <= 0 {
        return false;
    }
    match *cursor {
        None => *cursor = Some(result),
        Some(held) if same(held, result) => {
            let total = i16::from(held.count) + i16::from(result.count);
            if total > i16::from(MAX_STACK) {
                return false;
            }
            *cursor = Some(ItemStack::new(held.id, total as i8, held.damage));
        }
        Some(_) => return false,
    }
    true
}

/// Merge a stack into matching stacks and then empty slots. Any remainder is
/// returned to the caller, so callers can avoid silently deleting items.
pub fn insert_stack(slots: &mut [Option<ItemStack>], item: ItemStack) -> Option<ItemStack> {
    let mut remaining = i16::from(item.count);
    if remaining <= 0 {
        return None;
    }

    for existing in slots.iter_mut().flatten() {
        if !same(*existing, item) {
            continue;
        }
        let room = i16::from(MAX_STACK) - i16::from(existing.count);
        let moved = remaining.min(room.max(0));
        existing.count = (i16::from(existing.count) + moved) as i8;
        remaining -= moved;
        if remaining == 0 {
            return None;
        }
    }

    for slot in slots.iter_mut() {
        if slot.is_some() {
            continue;
        }
        let moved = remaining.min(i16::from(MAX_STACK));
        *slot = Some(ItemStack::new(item.id, moved as i8, item.damage));
        remaining -= moved;
        if remaining == 0 {
            return None;
        }
    }

    Some(ItemStack::new(item.id, remaining as i8, item.damage))
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

    #[test]
    fn player_and_table_grids_use_the_shared_recipe_book() {
        let mut player = vec![None; 45];
        player[4] = stack(17, 2);
        assert_eq!(player_crafting_result(&player), stack(5, 4));

        let mut table = vec![None; 46];
        table[1] = stack(5, 1);
        table[2] = stack(5, 1);
        table[3] = stack(5, 1);
        table[5] = stack(280, 1);
        table[8] = stack(280, 1);
        assert_eq!(table_crafting_result(&table), stack(270, 1));
    }

    #[test]
    fn crafting_consumes_one_item_from_each_filled_cell() {
        let mut grid = [stack(17, 2), None, stack(5, 1)];
        consume_crafting_grid(&mut grid);
        assert_eq!(grid, [stack(17, 1), None, None]);
    }

    #[test]
    fn crafting_output_is_atomic_with_respect_to_cursor_capacity() {
        let output = ItemStack::new(5, 4, 0);
        let mut cursor = None;
        assert!(take_crafting_result(&mut cursor, output));
        assert_eq!(cursor, Some(output));

        let mut full = stack(5, 62);
        assert!(!take_crafting_result(&mut full, output));
        assert_eq!(full, stack(5, 62));

        let mut incompatible = stack(17, 1);
        assert!(!take_crafting_result(&mut incompatible, output));
        assert_eq!(incompatible, stack(17, 1));
    }

    #[test]
    fn insert_stack_merges_before_using_an_empty_slot() {
        let mut slots = [stack(5, 62), None];
        assert_eq!(insert_stack(&mut slots, ItemStack::new(5, 4, 0)), None);
        assert_eq!(slots, [stack(5, 64), stack(5, 2)]);

        let mut full = [stack(5, 64)];
        assert_eq!(
            insert_stack(&mut full, ItemStack::new(5, 1, 0)),
            stack(5, 1)
        );
    }
}
