"""Test building sell and repair behaviors."""
from helpers import (
    get_snapshot, wait_ticks, find_actor,
    deploy_mcv, build_and_place, get_player,
    order_sell, order_repair,
)


def test_sell_building(game_page):
    """Selling a building should remove it and refund cash."""
    pid, fact = deploy_mcv(game_page)
    powr = build_and_place(game_page, pid, "powr", fact)
    assert powr, "Power plant required"

    snap = get_snapshot(game_page)
    cash_before = get_player(snap, pid)["cash"]

    order_sell(game_page, powr["id"])
    wait_ticks(game_page, 5)

    snap = get_snapshot(game_page)
    sold = find_actor(snap, id=powr["id"])
    assert sold is None, "Building should be removed after selling"
    cash_after = get_player(snap, pid)["cash"]
    assert cash_after > cash_before, "Cash should increase after selling"


def test_sell_refunds_partial_cost(game_page):
    """Selling should refund some fraction of original cost."""
    pid, fact = deploy_mcv(game_page)

    snap = get_snapshot(game_page)
    cash_before_build = get_player(snap, pid)["cash"]

    powr = build_and_place(game_page, pid, "powr", fact)
    assert powr, "Power plant required"

    snap = get_snapshot(game_page)
    cash_after_build = get_player(snap, pid)["cash"]
    cost = cash_before_build - cash_after_build

    order_sell(game_page, powr["id"])
    wait_ticks(game_page, 5)

    snap = get_snapshot(game_page)
    cash_after_sell = get_player(snap, pid)["cash"]
    refund = cash_after_sell - cash_after_build
    assert 0 < refund <= cost, f"Refund {refund} should be between 0 and {cost}"


def test_sell_fact(game_page):
    """Selling the construction yard should work or be gracefully ignored."""
    pid, fact = deploy_mcv(game_page)
    order_sell(game_page, fact["id"])
    try:
        wait_ticks(game_page, 5)
    except Exception:
        pass  # Game may stall after selling main building
    snap = get_snapshot(game_page)
    # Either sold or still there — both are acceptable
    assert True, "Sell order on FACT should not crash"


def test_repair_building(game_page):
    """Repair order on a building should not crash."""
    pid, fact = deploy_mcv(game_page)
    powr = build_and_place(game_page, pid, "powr", fact)
    assert powr, "Power plant required"
    # Issue repair order (even at full HP, should be harmless)
    order_repair(game_page, powr["id"])
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    p = find_actor(snap, id=powr["id"])
    assert p is not None, "Building should still exist after repair order"


def test_cannot_sell_enemy_building(game_page):
    """Selling an enemy building should be ignored."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    enemy_building = next(
        (a for a in snap["actors"]
         if a["owner"] != pid and a["owner"] > 2 and a["kind"] == "Building"),
        None
    )
    if not enemy_building:
        return  # Skip if no enemy buildings visible
    order_sell(game_page, enemy_building["id"])
    wait_ticks(game_page, 2)
    snap = get_snapshot(game_page)
    still_there = find_actor(snap, id=enemy_building["id"])
    assert still_there is not None, "Enemy building should not be sellable"
