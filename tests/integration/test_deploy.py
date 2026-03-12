"""Test MCV deployment behaviors."""
from helpers import (
    start_game, get_snapshot, wait_ticks, click_cell,
    find_actor, order_deploy, get_selected_units,
)


def test_deploy_via_keyboard(game_page):
    """Select MCV, press D should deploy to FACT."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    click_cell(game_page, mcv["x"], mcv["y"])
    sel = get_selected_units(game_page)
    assert mcv["id"] in sel, "MCV should be selected"
    game_page.keyboard.press("d")
    wait_ticks(game_page, 40)
    snap = get_snapshot(game_page)
    assert find_actor(snap, kind="Mcv", owner=pid) is None, "MCV should be gone"
    assert find_actor(snap, actor_type="fact", owner=pid) is not None, "FACT should exist"


def test_deploy_via_direct_order(game_page):
    """Direct order_deploy should also work."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    order_deploy(game_page, mcv["id"])
    wait_ticks(game_page, 40)
    snap = get_snapshot(game_page)
    assert find_actor(snap, actor_type="fact", owner=pid) is not None


def test_cannot_deploy_non_mcv(game_page):
    """Deploy order on non-MCV should be ignored."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    # Try to deploy a tree or mine (should be no-op)
    tree = next((a for a in snap["actors"] if a["kind"] == "Tree"), None)
    if tree:
        order_deploy(game_page, tree["id"])
        wait_ticks(game_page, 5)
        snap = get_snapshot(game_page)
        # Tree should still exist
        assert find_actor(snap, id=tree["id"]) is not None


def test_mcv_gone_after_deploy(game_page):
    """After deployment, there should be exactly 0 MCVs for the player."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    order_deploy(game_page, mcv["id"])
    wait_ticks(game_page, 40)
    snap = get_snapshot(game_page)
    mcvs = [a for a in snap["actors"] if a["kind"] == "Mcv" and a["owner"] == pid]
    assert len(mcvs) == 0, "No MCVs should remain after deploy"
