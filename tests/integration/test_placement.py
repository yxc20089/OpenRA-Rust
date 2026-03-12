"""Test building placement behaviors."""
from helpers import (
    start_game, get_snapshot, wait_ticks, find_actor, find_actors,
    order_start_production, order_place_building, can_place_building,
    deploy_mcv, build_and_place, get_ui_state, get_player,
)


def test_placement_valid_cell(game_page):
    """Placing building on valid cell should create it."""
    pid, fact = deploy_mcv(game_page)
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    place_x = fact["x"] + 3
    place_y = fact["y"]
    order_place_building(game_page, "powr", place_x, place_y)
    wait_ticks(game_page, 2)
    snap = get_snapshot(game_page)
    powr = find_actor(snap, actor_type="powr", owner=pid)
    assert powr is not None, "Power plant should be placed"


def test_placement_invalid_cell(game_page):
    """Placing on occupied cell should fail."""
    pid, fact = deploy_mcv(game_page)
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    # Try placing on top of the FACT itself
    result = can_place_building(game_page, "powr", fact["x"], fact["y"])
    assert result is False, "Should not be able to place on occupied cell"


def test_placement_adjacent_required(game_page):
    """Placing far from base should fail."""
    pid, fact = deploy_mcv(game_page)
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    # Place far away from base
    result = can_place_building(game_page, "powr", fact["x"] + 30, fact["y"] + 30)
    assert result is False, "Should not be able to place far from base"


def test_cancel_placement_escape(game_page):
    """Pressing Escape during placement mode should cancel it."""
    pid, fact = deploy_mcv(game_page)
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    # Check if placement mode was entered
    ui = get_ui_state(game_page)
    if ui.get("placementMode"):
        game_page.keyboard.press("Escape")
        ui = get_ui_state(game_page)
        assert ui.get("placementMode") is None, "Escape should cancel placement"


def test_placement_updates_power(game_page):
    """Placing a power plant should increase power provided."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    player_before = get_player(snap, pid)
    power_before = player_before.get("power_provided", 0)

    powr = build_and_place(game_page, pid, "powr", fact, offset_x=3, offset_y=0)
    assert powr, "Power plant should exist"

    snap = get_snapshot(game_page)
    player_after = get_player(snap, pid)
    power_after = player_after.get("power_provided", 0)
    assert power_after > power_before, f"Power should increase: {power_before} -> {power_after}"


def test_place_multiple_buildings(game_page):
    """Should be able to place buildings in different positions around base."""
    pid, fact = deploy_mcv(game_page)
    powr = build_and_place(game_page, pid, "powr", fact, offset_x=3, offset_y=0)
    assert powr, "First building should place"
    powr2 = build_and_place(game_page, pid, "powr", fact, offset_x=-3, offset_y=0)
    assert powr2, "Second building should place at different location"


def test_production_queue_shows_done(game_page):
    """Completed building should show done=true in queue."""
    pid, fact = deploy_mcv(game_page)
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    snap = get_snapshot(game_page)
    player = get_player(snap, pid)
    queue = player.get("production_queue", [])
    done_items = [q for q in queue if q["item_name"] == "powr" and q["done"]]
    assert len(done_items) >= 1, "Completed building should show done in queue"
