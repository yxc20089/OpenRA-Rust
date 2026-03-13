"""Test attack and combat behaviors."""
from helpers import (
    start_game, get_snapshot, wait_ticks, find_actor, find_actors,
    order_attack, order_stop, order_move, deploy_mcv,
    deploy_and_build_base, order_start_production, build_and_place,
)


def _get_enemy(snap, pid):
    """Find an enemy actor."""
    return next((a for a in snap["actors"] if a["owner"] != pid and a["owner"] > 2 and a["kind"] in ("Mcv", "Vehicle", "Infantry", "Building")), None)


def test_attack_order_changes_activity(game_page):
    """Issuing attack order should set activity to attacking."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    enemy = _get_enemy(snap, pid)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit or not enemy:
        return  # No unit/enemy for combat test
    order_attack(game_page, unit["id"], enemy["id"])
    wait_ticks(game_page, 10)
    snap = get_snapshot(game_page)
    u = find_actor(snap, id=unit["id"])
    assert u, "Attacker should still exist"
    assert u["activity"] in ("attacking", "moving"), f"Should be attacking or moving to target, got {u['activity']}"


def test_attack_reduces_hp(game_page):
    """Attacking an enemy should reduce its HP over time."""
    pid, fact, barr, weap = deploy_and_build_base(game_page)
    # Build an APC (soviet player)
    order_start_production(game_page, "apc")
    wait_ticks(game_page, 400)
    snap = get_snapshot(game_page)
    tank = next((a for a in snap["actors"] if a["owner"] == pid and a["actor_type"] == "apc"), None)
    enemy = _get_enemy(snap, pid)
    if not tank or not enemy:
        return
    initial_hp = enemy["hp"]
    order_attack(game_page, tank["id"], enemy["id"])
    wait_ticks(game_page, 100)
    snap = get_snapshot(game_page)
    e = find_actor(snap, id=enemy["id"])
    if e:
        assert e["hp"] < initial_hp, f"Enemy HP should decrease: {initial_hp} -> {e['hp']}"
    # If enemy is gone, attack killed it — also valid


def test_stop_attacking(game_page):
    """Stop order should cancel attack activity."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    enemy = _get_enemy(snap, pid)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit or not enemy:
        return
    order_attack(game_page, unit["id"], enemy["id"])
    wait_ticks(game_page, 5)
    order_stop(game_page, unit["id"])
    wait_ticks(game_page, 2)
    snap = get_snapshot(game_page)
    u = find_actor(snap, id=unit["id"])
    assert u and u["activity"] == "idle", f"Should be idle after stop, got {u['activity'] if u else 'None'}"


def test_attack_move_mode(game_page):
    """Press A, then right-click should move unit with attack-move behavior."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] == "Vehicle"), None)
    if not unit:
        return
    from helpers import click_cell, right_click_cell, get_ui_state
    click_cell(game_page, unit["x"], unit["y"])
    game_page.keyboard.press("a")
    ui = get_ui_state(game_page)
    assert ui["commandMode"] == "attack-move", f"Should be in attack-move mode"
    right_click_cell(game_page, unit["x"] + 5, unit["y"])
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    u = find_actor(snap, id=unit["id"])
    assert u and u["activity"] in ("moving", "attacking"), f"Should be moving/attacking, got {u['activity']}"


def test_sell_building(game_page):
    """Selling a building should remove it and refund cash."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    cash_before = next(p for p in snap["players"] if p["index"] == pid)["cash"]

    powr = build_and_place(game_page, pid, "powr", fact)
    assert powr, "Power plant should exist"

    snap = get_snapshot(game_page)
    cash_after_build = next(p for p in snap["players"] if p["index"] == pid)["cash"]
    assert cash_after_build < cash_before, "Cash should decrease after building"

    order_stop(game_page, powr["id"])  # just to ensure not doing anything
    from helpers import order_sell
    order_sell(game_page, powr["id"])
    wait_ticks(game_page, 2)

    snap = get_snapshot(game_page)
    sold = find_actor(snap, id=powr["id"])
    assert sold is None, "Building should be removed after selling"
    cash_after_sell = next(p for p in snap["players"] if p["index"] == pid)["cash"]
    assert cash_after_sell > cash_after_build, "Cash should increase after selling"
