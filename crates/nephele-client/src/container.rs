//! Local crafting-table container state and click rules.
//!
//! The player's inventory is owned by the game loop; this container owns only
//! its 3x3 grid and cursor. Shift-click transfers are returned to the caller so
//! it can insert them into the player inventory atomically.

use nephele_protocol::packets::play::ItemStack;

const GRID_CELLS: usize = 9;
const MAX_STACK: i16 = 64;

/// Which container window is open, so the UI draws the matching panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerKind {
    CraftingTable,
    Furnace,
    Chest,
}

/// Ticks a furnace needs per smelted item (10 s at 20 tps).
pub const FURNACE_COOK_TICKS: u32 = 200;

/// Fuel burn times, in ticks, for the fuels 1.8 accepts.
pub fn fuel_ticks(id: u16) -> Option<u32> {
    Some(match id {
        263 => 1600,  // coal
        173 => 16000, // coal block
        327 => 20000, // lava bucket
        // Planks, logs, wood stairs/slabs/fences, chests, crafting tables.
        5 | 17 | 25 | 47 | 54 | 58 | 85 | 126 | 134 | 135 | 136 | 163 | 164 => 300,
        280 => 100, // stick
        50 => 80,   // torch
        81 => 20,   // cactus
        _ => return None,
    })
}

/// A local furnace: input, fuel and output plus the two progress timers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LocalFurnace {
    pub input: Option<ItemStack>,
    pub fuel: Option<ItemStack>,
    pub output: Option<ItemStack>,
    /// Ticks of burn left in the current fuel item.
    pub burn: u32,
    /// Total burn time of the fuel item being consumed.
    pub burn_total: u32,
    /// Ticks of cooking done on the current input item.
    pub cook: u32,
}

impl LocalFurnace {
    pub fn flame(&self) -> f32 {
        if self.burn_total == 0 {
            0.0
        } else {
            self.burn as f32 / self.burn_total as f32
        }
    }

    pub fn progress(&self) -> f32 {
        self.cook as f32 / FURNACE_COOK_TICKS as f32
    }

    /// Whether `input` has a smelting result.
    pub fn result_for(input: ItemStack) -> Option<ItemStack> {
        let recipe = nephele_world::smelting::smelt(input.id as u16, input.damage as u16)?;
        Some(ItemStack::new(
            recipe.id as i16,
            recipe.count as i8,
            recipe.damage as i16,
        ))
    }

    fn can_accept(&self, result: ItemStack) -> bool {
        match self.output {
            None => true,
            Some(out) => out.id == result.id && out.damage == result.damage && out.count < 64,
        }
    }

    /// Advance one tick: consume fuel, cook, and produce output.
    pub fn tick(&mut self) {
        let result = self.input.and_then(Self::result_for);
        let cooking = result.is_some_and(|result| self.can_accept(result));

        if self.burn > 0 {
            self.burn -= 1;
        } else if cooking {
            // Light the next fuel item.
            if let Some(fuel) = self.fuel {
                if let Some(ticks) = fuel_ticks(fuel.id as u16) {
                    self.burn = ticks;
                    self.burn_total = ticks;
                    let left = fuel.count - 1;
                    self.fuel = if left > 0 {
                        Some(ItemStack::new(fuel.id, left, fuel.damage))
                    } else {
                        None
                    };
                }
            }
        }

        if cooking && self.burn > 0 {
            self.cook += 1;
            if self.cook >= FURNACE_COOK_TICKS {
                self.cook = 0;
                if let Some(result) = result {
                    self.output = Some(match self.output {
                        None => result,
                        Some(out) => ItemStack::new(out.id, out.count + result.count, out.damage),
                    });
                    if let Some(input) = self.input {
                        let left = input.count - 1;
                        self.input = if left > 0 {
                            Some(ItemStack::new(input.id, left, input.damage))
                        } else {
                            None
                        };
                    }
                }
            }
        } else {
            self.cook = 0;
        }
    }
}

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
    fn furnace_smelts_iron_ore_into_an_ingot() {
        let mut furnace = LocalFurnace {
            input: stack(15, 1),
            fuel: stack(263, 1),
            ..LocalFurnace::default()
        };
        // One coal burns for 1600 ticks; the item itself takes 200.
        for _ in 0..FURNACE_COOK_TICKS {
            furnace.tick();
        }
        assert_eq!(furnace.output, Some(ItemStack::new(265, 1, 0)));
        assert_eq!(furnace.input, None);
        assert!(furnace.burn > 0, "the coal is still burning");
        assert_eq!(furnace.progress(), 0.0);
    }

    #[test]
    fn furnace_does_nothing_without_fuel() {
        let mut furnace = LocalFurnace {
            input: stack(15, 1),
            ..LocalFurnace::default()
        };
        for _ in 0..FURNACE_COOK_TICKS {
            furnace.tick();
        }
        assert_eq!(furnace.output, None);
        assert_eq!(furnace.input, stack(15, 1));
        assert_eq!(furnace.flame(), 0.0);
    }

    #[test]
    fn furnace_stops_when_the_output_is_full() {
        let mut furnace = LocalFurnace {
            input: stack(15, 2),
            fuel: stack(263, 1),
            output: stack(265, 64),
            ..LocalFurnace::default()
        };
        for _ in 0..(FURNACE_COOK_TICKS * 2) {
            furnace.tick();
        }
        assert_eq!(furnace.input, stack(15, 2), "nothing may be destroyed");
        assert_eq!(furnace.output, stack(265, 64));
    }

    #[test]
    fn furnace_flame_follows_the_burn_timer() {
        let mut furnace = LocalFurnace {
            input: stack(15, 1),
            fuel: stack(263, 1),
            ..LocalFurnace::default()
        };
        furnace.tick();
        assert_eq!(furnace.flame(), 1.0);
        for _ in 0..(1600 / 2) {
            furnace.tick();
        }
        assert!(furnace.flame() > 0.3 && furnace.flame() < 0.7);
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
