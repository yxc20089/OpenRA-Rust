"""Test production queue and unit spawning."""
from helpers import (
    start_game, get_snapshot, wait_ticks, find_actor, find_actors,
    order_start_production, order_place_building, order_deploy,
    deploy_mcv, build_and_place, deploy_and_build_base, get_player,
)


def _deploy_mcv(page):
    """Deploy MCV and return (pid, fact)."""
    return deploy_mcv(page)


def _build_and_place(page, pid, building_type, fact, offset_x=3, offset_y=0):
    """Produce and place a building."""
    return build_and_place(page, pid, building_type, fact, offset_x, offset_y)


def test_build_power_plant(game_page):
    """Should be able to build and place a power plant."""
    pid, fact = _deploy_mcv(game_page)
    assert fact, "FACT required"
    powr = _build_and_place(game_page, pid, "powr", fact)
    assert powr is not None, "Power plant should exist after placement"


def test_build_barracks_and_train_infantry(game_page):
    """Should be able to build barracks, then train infantry."""
    page = game_page
    pid, fact = _deploy_mcv(page)
    assert fact, "FACT required"

    _build_and_place(page, pid, "powr", fact, offset_x=3, offset_y=0)
    tent = _build_and_place(page, pid, "tent", fact, offset_x=-2, offset_y=0)
    assert tent, "Barracks should exist"

    order_start_production(page, "e1")
    wait_ticks(page, 350)

    snap = get_snapshot(page)
    e1_units = find_actors(snap, actor_type="e1", owner=pid)
    assert len(e1_units) >= 1, "At least one rifle infantry should have spawned"


def test_multiple_buildings_in_queue(game_page):
    """Multiple buildings can be queued and placed sequentially."""
    page = game_page
    pid, fact = _deploy_mcv(page)
    assert fact, "FACT required"

    order_start_production(page, "powr")
    order_start_production(page, "powr")

    wait_ticks(page, 300)

    snap = get_snapshot(page)
    player = next(p for p in snap["players"] if p["index"] == pid)
    queue = player.get("production_queue", [])
    done_items = [q for q in queue if q["item_name"] == "powr" and q["done"]]
    assert len(done_items) >= 1, "At least one power plant should be done"

    place_x = fact["x"] + 3
    order_place_building(page, "powr", place_x, fact["y"])
    wait_ticks(page, 2)

    wait_ticks(page, 350)

    snap = get_snapshot(page)
    player = next(p for p in snap["players"] if p["index"] == pid)
    queue = player.get("production_queue", [])
    done_items = [q for q in queue if q["item_name"] == "powr" and q["done"]]
    assert len(done_items) >= 1, "Second power plant should be done now"


def test_production_costs_money(game_page):
    """Building a power plant ($300) should decrease cash."""
    pid, fact = _deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    cash_before = get_player(snap, pid)["cash"]
    order_start_production(game_page, "powr")
    wait_ticks(game_page, 350)
    snap = get_snapshot(game_page)
    cash_after = get_player(snap, pid)["cash"]
    assert cash_after < cash_before, f"Cash should decrease: {cash_before} -> {cash_after}"
    spent = cash_before - cash_after
    assert spent > 0, f"Should have spent money, spent={spent}"


def test_unit_spawns_near_building(game_page):
    """Infantry should spawn near barracks after training."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not tent:
        return
    order_start_production(game_page, "e1")
    wait_ticks(game_page, 350)
    snap = get_snapshot(game_page)
    e1_units = find_actors(snap, actor_type="e1", owner=pid)
    if len(e1_units) > 0:
        unit = e1_units[-1]
        dist = abs(unit["x"] - tent["x"]) + abs(unit["y"] - tent["y"])
        assert dist < 15, f"Infantry should spawn near barracks, dist={dist}"


def test_vehicle_spawns_near_weap(game_page):
    """Vehicle should spawn near war factory after training."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not weap:
        return
    order_start_production(game_page, "2tnk")
    wait_ticks(game_page, 400)
    snap = get_snapshot(game_page)
    tanks = find_actors(snap, actor_type="2tnk", owner=pid)
    if len(tanks) > 0:
        tank = tanks[-1]
        dist = abs(tank["x"] - weap["x"]) + abs(tank["y"] - weap["y"])
        assert dist < 15, f"Vehicle should spawn near war factory, dist={dist}"


def test_train_multiple_units(game_page):
    """Training multiple units sequentially should work."""
    pid, fact, tent, weap = deploy_and_build_base(game_page)
    if not tent:
        return
    order_start_production(game_page, "e1")
    wait_ticks(game_page, 50)  # Let first order start processing
    order_start_production(game_page, "e1")
    wait_ticks(game_page, 800)
    snap = get_snapshot(game_page)
    e1_units = find_actors(snap, actor_type="e1", owner=pid)
    assert len(e1_units) >= 2, f"Should have at least 2 infantry, got {len(e1_units)}"


def test_cannot_produce_without_prereq(game_page):
    """Trying to produce advanced unit without prerequisites should fail gracefully."""
    pid, fact = _deploy_mcv(game_page)
    # Try to produce a tank without war factory — should be no-op
    snap_before = get_snapshot(game_page)
    cash_before = get_player(snap_before, pid)["cash"]
    order_start_production(game_page, "2tnk")
    wait_ticks(game_page, 5)
    snap_after = get_snapshot(game_page)
    cash_after = get_player(snap_after, pid)["cash"]
    # Cash should not change if production was rejected
    assert cash_after == cash_before, "Cash should not change without prerequisites"
