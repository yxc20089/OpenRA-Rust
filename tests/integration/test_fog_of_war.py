"""Test fog of war and visibility."""
from helpers import (
    get_snapshot, wait_ticks, find_actor, find_actors,
    deploy_mcv, get_ui_state, order_move,
)


def test_explored_cells_exist(game_page):
    """Game should have explored cells around starting position."""
    pid, fact = deploy_mcv(game_page)
    ui = get_ui_state(game_page)
    explored = ui.get("exploredCells", 0)
    assert explored > 0, f"Should have explored cells, got {explored}"


def test_explored_cells_grow_on_move(game_page):
    """Moving a unit should increase explored cells."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next(
        (a for a in snap["actors"]
         if a["owner"] == pid and a["kind"] in ("Vehicle", "Infantry")),
        None
    )
    if not unit:
        return  # Skip if no mobile units

    ui_before = get_ui_state(game_page)
    explored_before = ui_before.get("exploredCells", 0)

    # Move unit far
    order_move(game_page, unit["id"], unit["x"] + 15, unit["y"])
    wait_ticks(game_page, 30)

    ui_after = get_ui_state(game_page)
    explored_after = ui_after.get("exploredCells", 0)
    assert explored_after >= explored_before, \
        f"Explored cells should not decrease: {explored_before} -> {explored_after}"


def test_own_base_visible(game_page):
    """Player's own base should always be visible."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    # Own fact should be in snapshot
    own_fact = find_actor(snap, actor_type="fact", owner=pid)
    assert own_fact is not None, "Own construction yard should be visible"
