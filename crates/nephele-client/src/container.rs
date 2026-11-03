//! Local crafting-table container state and click rules.
//!
//! The player's inventory is owned by the game loop; this container owns only
//! its 3x3 grid and cursor. Shift-click transfers are returned to the caller so
//! it can insert them into the player inventory atomically.

use nephele_protocol::packets::play::ItemStack;

const GRID_CELLS: usize = 9;
const MAX_STACK: i16 = 64;

/// A local 3x3 crafting-table window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalContainer {
    grid: [Option<ItemStack>; GRID_CELLS],
    cursor: Option<ItemStack>,
}

impl LocalContainer {
    /// The nine crafting cells, in row-major order.
    pub fn grid(&self) -> &[Option<ItemStack>; GRID_CELLS] {
        &self.grid
    }

    /// The stack currently carried by the cursor.
    pub fn cursor(&self) -> Option<ItemStack> {
        self.cursor
    }

    /// Set the cursor while moving a stack to or from the player inventory.
    pub fn set_cursor(&mut self, cursor: Option<ItemStack>) {
        self.cursor = cursor;
    }

    /// Set a crafting cell by its zero-based row-major index.
    ///
    /// Returns `false` for an index outside the 3x3 grid.
    pub fn set_grid_cell(&mut self, index: usize, item: Option<ItemStack>) -> bool {
        let Some(cell) = self.grid.get_mut(index) else {
            return false;
        };
        *cell = item;
        true
    }

    /// The recipe currently shown in the result slot, if any.
    pub fn result(&self) -> Option<ItemStack> {
        if self
            .grid
            .iter()
            .flatten()
            .any(|item| item.id < 0 || item.count <= 0)
        {
            return None;
        }

        let ingredients = std::array::from_fn(|index| {
            self.grid[index].map(|item| (item.id as u16, item.damage.max(0) as u16))
        });
        let recipe = nephele_world::crafting::craft(&ingredients)?;
        Some(ItemStack::new(
            recipe.id as i16,
            recipe.count as i8,
            recipe.damage as i16,
        ))
    }

    /// Apply a click to a table-window slot: 0 is the result and 1..=9 are the
    /// crafting cells. `button` is 0 for left-click and 1 for right-click.
    ///
    /// Normal result clicks put the full recipe output on the cursor and spend
    /// one item from every occupied cell only when that whole output fits.
    /// Normal cell clicks use the usual take/place/merge/swap rules. A
    /// shift-click returns the moved stack; for a result click it also spends
    /// one recipe from the grid. The caller should perform shift-clicks on a
    /// clone and commit only if the player inventory accepts the returned
    /// stack.
    pub fn click(&mut self, slot: usize, button: i8, shift: bool) -> Option<ItemStack> {
        if button != 0 && button != 1 {
            return None;
        }

        if slot == 0 {
            let result = self.result()?;
            if shift {
                self.spend_grid();
                return Some(result);
            }
            if take_result(&mut self.cursor, result) {
                self.spend_grid();
            }
            return None;
        }

        let cell_index = slot.checked_sub(1)?;
        let cell = self.grid.get_mut(cell_index)?;
        if shift {
            return cell.take();
        }

        let (cursor, new_cell) = click_stack(self.cursor, *cell, button);
        self.cursor = cursor;
        *cell = new_cell;
        None
    }

    /// Spend one item from every occupied crafting cell, like the mock's table
    /// result handling.
    fn spend_grid(&mut self) {
        for cell in &mut self.grid {
            if let Some(mut stack) = *cell {
                stack.count -= 1;
                *cell = (stack.count > 0).then_some(stack);
            }
        }
    }
}

fn same_item(a: ItemStack, b: ItemStack) -> bool {
    a.id == b.id && a.damage == b.damage
}

fn click_stack(
    cursor: Option<ItemStack>,
    slot: Option<ItemStack>,
    button: i8,
) -> (Option<ItemStack>, Option<ItemStack>) {
    match (cursor, slot, button) {
        // Left click: take all, place all, merge, or swap.
        (None, Some(on_slot), 0) => (Some(on_slot), None),
        (Some(held), None, 0) => (None, Some(held)),
        (Some(held), Some(on_slot), 0)
            if same_item(held, on_slot) && on_slot.count < MAX_STACK as i8 =>
        {
            let moved = i16::from(held.count).min(MAX_STACK - i16::from(on_slot.count));
            let left = i16::from(held.count) - moved;
            (
                (left > 0).then(|| ItemStack::new(held.id, left as i8, held.damage)),
                Some(ItemStack::new(
                    on_slot.id,
                    (i16::from(on_slot.count) + moved) as i8,
                    on_slot.damage,
                )),
            )
        }
        (Some(held), Some(on_slot), 0) => (Some(on_slot), Some(held)),

        // Right click: take the rounded-up half, or place one item.
        (None, Some(on_slot), 1) if on_slot.count > 0 => {
            let take = (i16::from(on_slot.count) + 1) / 2;
            let left = i16::from(on_slot.count) - take;
            (
                Some(ItemStack::new(on_slot.id, take as i8, on_slot.damage)),
                (left > 0).then(|| ItemStack::new(on_slot.id, left as i8, on_slot.damage)),
            )
        }
        (Some(held), None, 1) if held.count > 0 => {
            let left = i16::from(held.count) - 1;
            (
                (left > 0).then(|| ItemStack::new(held.id, left as i8, held.damage)),
                Some(ItemStack::new(held.id, 1, held.damage)),
            )
        }
        (Some(held), Some(on_slot), 1)
            if same_item(held, on_slot) && on_slot.count < MAX_STACK as i8 && held.count > 0 =>
        {
            let left = held.count - 1;
            (
                (left > 0).then(|| ItemStack::new(held.id, left, held.damage)),
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

fn take_result(cursor: &mut Option<ItemStack>, result: ItemStack) -> bool {
    if result.count <= 0 {
        return false;
    }
    match *cursor {
        None => *cursor = Some(result),
        Some(held) if same_item(held, result) => {
            let total = i16::from(held.count) + i16::from(result.count);
            if total > MAX_STACK {
                return false;
            }
            *cursor = Some(ItemStack::new(held.id, total as i8, held.damage));
        }
        Some(_) => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: i16, count: i8) -> Option<ItemStack> {
        Some(ItemStack::new(id, count, 0))
    }

    #[test]
    fn left_click_takes_places_merges_and_swaps() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(17, 3));
        container.click(1, 0, false);
        assert_eq!(container.cursor(), stack(17, 3));
        assert_eq!(container.grid()[0], None);

        container.click(2, 0, false);
        assert_eq!(container.cursor(), None);
        assert_eq!(container.grid()[1], stack(17, 3));

        container.set_cursor(stack(5, 2));
        container.click(2, 0, false);
        assert_eq!(container.cursor(), stack(17, 3));
        assert_eq!(container.grid()[1], stack(5, 2));
    }

    #[test]
    fn left_click_merges_only_to_stack_limit_and_keeps_overflow() {
        let mut container = LocalContainer::default();
        container.set_cursor(stack(5, 60));
        container.set_grid_cell(0, stack(5, 10));
        container.click(1, 0, false);
        assert_eq!(container.cursor(), stack(5, 6));
        assert_eq!(container.grid()[0], stack(5, 64));

        container.set_cursor(stack(5, 2));
        container.set_grid_cell(0, stack(5, 62));
        container.click(1, 0, false);
        assert_eq!(container.cursor(), None);
        assert_eq!(container.grid()[0], stack(5, 64));
    }

    #[test]
    fn right_click_takes_half_places_one_and_merges_one() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(5, 5));
        container.click(1, 1, false);
        assert_eq!(container.cursor(), stack(5, 3));
        assert_eq!(container.grid()[0], stack(5, 2));

        container.click(2, 1, false);
        assert_eq!(container.cursor(), stack(5, 2));
        assert_eq!(container.grid()[1], stack(5, 1));

        container.set_grid_cell(1, stack(5, 1));
        container.click(2, 1, false);
        assert_eq!(container.cursor(), stack(5, 1));
        assert_eq!(container.grid()[1], stack(5, 2));
    }

    #[test]
    fn incompatible_and_full_stacks_do_not_merge_on_right_click() {
        let mut container = LocalContainer::default();
        container.set_cursor(stack(5, 2));
        container.set_grid_cell(0, stack(17, 3));
        container.click(1, 1, false);
        assert_eq!(container.cursor(), stack(5, 2));
        assert_eq!(container.grid()[0], stack(17, 3));

        container.set_grid_cell(0, stack(5, 64));
        container.click(1, 1, false);
        assert_eq!(container.cursor(), stack(5, 2));
        assert_eq!(container.grid()[0], stack(5, 64));
    }

    #[test]
    fn result_click_spends_one_item_from_each_occupied_cell() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(5, 3));
        container.set_grid_cell(3, stack(5, 2));
        assert_eq!(container.result(), stack(280, 4));

        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(280, 4));
        assert_eq!(container.grid()[0], stack(5, 2));
        assert_eq!(container.grid()[3], stack(5, 1));

        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(280, 8));
        assert_eq!(container.grid()[0], stack(5, 1));
        assert_eq!(container.grid()[3], None);
    }

    #[test]
    fn result_click_is_atomic_when_the_cursor_cannot_hold_the_whole_output() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(17, 1));
        container.set_cursor(stack(5, 62));
        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(5, 62));
        assert_eq!(container.grid()[0], stack(17, 1));

        container.set_cursor(stack(280, 1));
        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(280, 1));
        assert_eq!(container.grid()[0], stack(17, 1));
    }

    #[test]
    fn shift_click_returns_grid_items_or_one_craft_output() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(5, 2));
        container.set_grid_cell(3, stack(5, 2));
        container.set_cursor(stack(17, 1));

        assert_eq!(container.click(1, 0, true), stack(5, 2));
        assert_eq!(container.grid()[0], None);
        assert_eq!(container.cursor(), stack(17, 1));

        container.set_grid_cell(0, stack(5, 2));
        let output = container.click(0, 0, true);
        assert_eq!(output, stack(280, 4));
        assert_eq!(container.grid()[0], stack(5, 1));
        assert_eq!(container.grid()[3], stack(5, 1));
        assert_eq!(container.cursor(), stack(17, 1));
    }

    #[test]
    fn invalid_slots_and_buttons_leave_the_container_unchanged() {
        let mut container = LocalContainer::default();
        container.set_cursor(stack(5, 2));
        container.set_grid_cell(0, stack(17, 3));
        assert_eq!(container.click(10, 0, false), None);
        assert_eq!(container.click(1, 2, false), None);
        assert_eq!(container.cursor(), stack(5, 2));
        assert_eq!(container.grid()[0], stack(17, 3));
        assert!(!container.set_grid_cell(9, stack(5, 1)));
    }

    #[test]
    fn two_logs_craft_into_a_wooden_pickaxe_through_table_clicks() {
        let mut container = LocalContainer::default();
        container.set_grid_cell(0, stack(17, 2));
        assert_eq!(container.result(), stack(5, 4));
        container.click(0, 0, false);
        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(5, 8));
        assert_eq!(container.grid()[0], None);

        // Move two planks from the cursor into a vertical pair for sticks.
        container.set_cursor(stack(5, 8));
        container.click(1, 1, false);
        container.click(4, 1, false);
        assert_eq!(container.cursor(), stack(5, 6));
        container.set_cursor(None); // The remaining planks are in the inventory.
        assert_eq!(container.result(), stack(280, 4));
        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(280, 4));
        assert_eq!(container.grid()[0], None);
        assert_eq!(container.grid()[3], None);

        // Place three planks across the top and two sticks down the centre.
        container.set_cursor(None); // The sticks were moved into the inventory.
        container.set_cursor(stack(5, 6));
        for slot in [1, 2, 3] {
            container.click(slot, 1, false);
        }
        assert_eq!(container.cursor(), stack(5, 3));
        container.set_cursor(None); // Keep the three spare planks in inventory.
        container.set_cursor(stack(280, 4));
        container.click(5, 1, false);
        container.click(8, 1, false);
        assert_eq!(container.cursor(), stack(280, 2));
        container.set_cursor(None);

        assert_eq!(container.result(), stack(270, 1));
        container.click(0, 0, false);
        assert_eq!(container.cursor(), stack(270, 1));
        assert!(container.grid().iter().all(Option::is_none));
    }
}
