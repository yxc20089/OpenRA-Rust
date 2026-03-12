"""Test unit movement behaviors."""
from helpers import (
    start_game, get_snapshot, wait_ticks, click_cell, right_click_cell,
    find_actor, find_actors, order_move, order_stop, deploy_mcv,
    get_selected_units, get_ui_state,
)


def test_right_click_moves_unit(game_page):
    """Right-clicking empty cell with unit selected should move it."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    # Find a mobile unit (harvester should exist after deploy)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] in ("Vehicle", "Infantry", "Mcv")), None)
    if not unit:
        # Use direct order on any vehicle
        return
    click_cell(game_page, unit["x"], unit["y"])
    target_x, target_y = unit["x"] + 3, unit["y"]
    right_click_cell(game_page, target_x, target_y)
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    moved = find_actor(snap, id=unit["id"])
    assert moved, "Unit should still exist"
    assert moved["activity"] == "moving" or (moved["x"] != unit["x"] or moved["y"] != unit["y"]), \
        f"Unit should be moving or have moved, activity={moved['activity']}"


def test_move_order_directly(game_page):
    """Direct order_move should change unit activity to moving."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit:
        return
    order_move(game_page, unit["id"], unit["x"] + 5, unit["y"])
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    moved = find_actor(snap, id=unit["id"])
    assert moved and moved["activity"] == "moving", f"Activity should be moving, got {moved['activity'] if moved else 'None'}"


def test_stop_moving_unit(game_page):
    """Pressing S should stop a moving unit."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit:
        return
    order_move(game_page, unit["id"], unit["x"] + 10, unit["y"])
    wait_ticks(game_page, 3)
    order_stop(game_page, unit["id"])
    wait_ticks(game_page, 2)
    snap = get_snapshot(game_page)
    stopped = find_actor(snap, id=unit["id"])
    assert stopped and stopped["activity"] == "idle", f"Unit should be idle after stop, got {stopped['activity'] if stopped else 'None'}"


def test_move_command_mode(game_page):
    """Press M to enter move mode, then right-click should move."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit:
        return
    click_cell(game_page, unit["x"], unit["y"])
    game_page.keyboard.press("m")
    ui = get_ui_state(game_page)
    assert ui["commandMode"] == "move", f"Should be in move mode, got {ui['commandMode']}"
    right_click_cell(game_page, unit["x"] + 3, unit["y"])
    wait_ticks(game_page, 5)
    ui = get_ui_state(game_page)
    assert ui["commandMode"] is None, "Command mode should reset after action"


def test_scatter_command(game_page):
    """Pressing X with units selected should scatter them."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    units = [a for a in snap["actors"] if a["owner"] == pid and a["kind"] in ("Vehicle", "Infantry")]
    if len(units) == 0:
        return
    # Select a unit
    u = units[0]
    click_cell(game_page, u["x"], u["y"])
    game_page.keyboard.press("x")
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    scattered = find_actor(snap, id=u["id"])
    assert scattered, "Unit should still exist after scatter"
    # Unit should be moving or have moved
    assert scattered["activity"] == "moving" or scattered["x"] != u["x"] or scattered["y"] != u["y"], \
        "Unit should scatter (move) after X"


def test_move_to_occupied_cell(game_page):
    """Moving to an occupied cell should not crash."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit:
        return
    # Move to fact's cell (occupied by building)
    order_move(game_page, unit["id"], fact["x"], fact["y"])
    wait_ticks(game_page, 10)
    snap = get_snapshot(game_page)
    u = find_actor(snap, id=unit["id"])
    assert u is not None, "Unit should still exist after moving to occupied cell"
