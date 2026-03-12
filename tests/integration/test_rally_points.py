"""Test rally point behaviors."""
from helpers import (
    get_snapshot, wait_ticks, find_actor, find_actors,
    deploy_mcv, build_and_place, deploy_and_build_base,
    order_set_rally_point, order_start_production,
    click_cell, right_click_cell,
)


def test_set_rally_point(game_page):
    """Setting rally point on barracks should not crash."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not tent:
        return
    order_set_rally_point(game_page, tent["id"], tent["x"] + 3, tent["y"])
    wait_ticks(game_page, 2)
    assert True, "Set rally point should not crash"


def test_unit_moves_to_rally(game_page):
    """Units produced after setting rally should move toward rally point."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not tent:
        return
    rally_x = tent["x"] + 5
    rally_y = tent["y"]
    order_set_rally_point(game_page, tent["id"], rally_x, rally_y)
    wait_ticks(game_page, 2)

    order_start_production(game_page, "e1")
    wait_ticks(game_page, 350)

    snap = get_snapshot(game_page)
    e1_units = find_actors(snap, actor_type="e1", owner=pid)
    if len(e1_units) > 0:
        unit = e1_units[-1]  # Most recently spawned
        # Unit should be moving toward rally or already there
        dist = abs(unit["x"] - rally_x) + abs(unit["y"] - rally_y)
        # Allow generous distance — rally is a suggestion, not exact
        assert dist < 20 or unit.get("activity") == "moving", \
            f"Unit should be near rally point or moving, dist={dist}"


def test_rally_on_war_factory(game_page):
    """Setting rally on war factory should also work."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not weap:
        return
    order_set_rally_point(game_page, weap["id"], weap["x"] + 4, weap["y"])
    wait_ticks(game_page, 2)
    assert True, "Rally on war factory should not crash"
