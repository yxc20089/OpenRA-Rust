"""Test camera controls and viewport."""
from helpers import start_game, get_cam


def test_zoom_in(game_page):
    """Pressing + should increase cell pixel size."""
    start_game(game_page)
    before = get_cam(game_page)["cellPx"]
    game_page.keyboard.press("=")  # + key
    after = get_cam(game_page)["cellPx"]
    assert after > before, f"cellPx should increase: {before} -> {after}"


def test_zoom_out(game_page):
    """Pressing - should decrease cell pixel size."""
    start_game(game_page)
    before = get_cam(game_page)["cellPx"]
    game_page.keyboard.press("-")
    after = get_cam(game_page)["cellPx"]
    assert after < before, f"cellPx should decrease: {before} -> {after}"


def test_zoom_clamp_max(game_page):
    """Zooming in should cap at 96px."""
    start_game(game_page)
    for _ in range(30):
        game_page.keyboard.press("=")
    assert get_cam(game_page)["cellPx"] <= 96


def test_zoom_clamp_min(game_page):
    """Zooming out should not go below 8px."""
    start_game(game_page)
    for _ in range(30):
        game_page.keyboard.press("-")
    assert get_cam(game_page)["cellPx"] >= 8


def test_minimap_click_pans(game_page):
    """Clicking the minimap should pan the camera."""
    start_game(game_page)
    before = get_cam(game_page)
    # Click top-left corner of minimap
    minimap = game_page.locator("#minimap-canvas")
    minimap.click(position={"x": 10, "y": 10})
    after = get_cam(game_page)
    # Camera should have moved (may or may not change both axes depending on map)
    moved = before["camX"] != after["camX"] or before["camY"] != after["camY"]
    assert moved, "Camera should pan when minimap is clicked"
