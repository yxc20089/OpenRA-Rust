"""Test MCV deployment and building placement flow."""
from helpers import (
    start_game, get_snapshot, wait_ticks, find_actor,
    order_start_production, order_place_building, order_deploy, can_place_building,
)


def test_game_starts_with_mcv(game_page):
    """Game should start with an MCV for the human player."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    assert mcv is not None, f"Human player {pid} should have an MCV at game start"


def test_mcv_deploy_creates_construction_yard(game_page):
    """Deploying MCV should create a Construction Yard."""
    page = game_page
    pid = start_game(page)

    snap = get_snapshot(page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    assert mcv, "No MCV found"

    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)

    snap = get_snapshot(page)
    mcv_after = find_actor(snap, kind="Mcv", owner=pid)
    assert mcv_after is None, "MCV should be gone after deployment"

    fact = find_actor(snap, actor_type="fact", owner=pid)
    assert fact is not None, "Construction Yard (fact) should exist after MCV deploy"


def test_production_queue_shows_done_building(game_page):
    """After producing a building, it should stay in queue as done=true."""
    page = game_page
    pid = start_game(page)

    snap = get_snapshot(page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)

    snap = get_snapshot(page)
    fact = find_actor(snap, actor_type="fact", owner=pid)
    assert fact, "FACT should exist"

    order_start_production(page, "powr")
    wait_ticks(page, 300)

    snap = get_snapshot(page)
    player = next(p for p in snap["players"] if p["index"] == pid)
    queue = player.get("production_queue", [])
    powr_item = next((q for q in queue if q["item_name"] == "powr"), None)
    assert powr_item is not None, f"Power plant should still be in queue. Queue: {queue}"
    assert powr_item["done"] is True, "Power plant should be marked as done"


def test_place_building_adjacent_to_fact(game_page):
    """Player should be able to place a completed building adjacent to FACT."""
    page = game_page
    pid = start_game(page)

    snap = get_snapshot(page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)

    snap = get_snapshot(page)
    fact = find_actor(snap, actor_type="fact", owner=pid)
    assert fact, "FACT should exist"

    order_start_production(page, "powr")
    wait_ticks(page, 300)

    place_x = fact["x"] + 3
    place_y = fact["y"]

    assert can_place_building(page, "powr", place_x, place_y), \
        f"Should be able to place powr at ({place_x},{place_y})"

    order_place_building(page, "powr", place_x, place_y)
    wait_ticks(page, 2)

    snap = get_snapshot(page)
    powr = find_actor(snap, actor_type="powr", owner=pid)
    assert powr is not None, "Power plant should be placed on the map"

    player = next(p for p in snap["players"] if p["index"] == pid)
    queue = player.get("production_queue", [])
    powr_in_queue = next((q for q in queue if q["item_name"] == "powr"), None)
    assert powr_in_queue is None, "Placed building should be removed from queue"


def test_screenshot_after_deploy(game_page):
    """Take a screenshot after MCV deploy for visual verification."""
    page = game_page
    pid = start_game(page)

    snap = get_snapshot(page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)

    page.screenshot(path="tests/integration/screenshots/after_mcv_deploy.png")
