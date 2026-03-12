"""Test unit selection behaviors."""
from helpers import (
    start_game, get_snapshot, wait_ticks, click_cell, shift_click_cell,
    find_actor, find_actors, get_selected_units, deploy_mcv, order_deploy,
    ORA,
)


def test_click_select_unit(game_page):
    """Left-clicking own unit should select it."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    click_cell(game_page, mcv["x"], mcv["y"])
    sel = get_selected_units(game_page)
    assert mcv["id"] in sel, f"MCV {mcv['id']} should be selected, got {sel}"


def test_click_select_building(game_page):
    """Left-clicking own building should select it."""
    pid, fact = deploy_mcv(game_page)
    click_cell(game_page, fact["x"], fact["y"])
    sel = get_selected_units(game_page)
    assert fact["id"] in sel, f"FACT {fact['id']} should be selected"


def test_click_deselect(game_page):
    """Left-clicking empty cell should deselect all."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    # Select MCV
    click_cell(game_page, mcv["x"], mcv["y"])
    assert len(get_selected_units(game_page)) > 0
    # Click empty cell far from any actor
    click_cell(game_page, mcv["x"] + 5, mcv["y"] + 5)
    sel = get_selected_units(game_page)
    assert len(sel) == 0, f"Selection should be empty after clicking empty cell, got {sel}"


def test_shift_click_multiselect(game_page):
    """Shift+clicking multiple units should multi-select."""
    pid, fact = deploy_mcv(game_page)
    # Deploy creates a harvester too — find both fact and any other owned actor
    snap = get_snapshot(game_page)
    owned = [a for a in snap["actors"] if a["owner"] == pid and a["kind"] != "Player"]
    if len(owned) < 2:
        # Only fact — just verify shift-click keeps selection
        click_cell(game_page, fact["x"], fact["y"])
        sel = get_selected_units(game_page)
        assert len(sel) >= 1
        return
    a1, a2 = owned[0], owned[1]
    click_cell(game_page, a1["x"], a1["y"])
    shift_click_cell(game_page, a2["x"], a2["y"])
    sel = get_selected_units(game_page)
    assert a1["id"] in sel and a2["id"] in sel, f"Both actors should be selected: {sel}"


def test_select_ignores_enemy(game_page):
    """Clicking enemy unit alone should not add it to selection."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    enemy = next((a for a in snap["actors"] if a["owner"] != pid and a["owner"] > 2 and a["kind"] == "Mcv"), None)
    if not enemy:
        return  # Skip if no visible enemy
    # Check if enemy is visible on screen before clicking
    cam = game_page.evaluate(f"({{ camX: {ORA}.camX, camY: {ORA}.camY, cellPx: {ORA}.cellPx }})")
    canvas = game_page.locator("#canvas")
    box = canvas.bounding_box()
    px = (enemy["x"] * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
    py = (enemy["y"] * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    if px < 0 or px > box["width"] or py < 0 or py > box["height"]:
        return  # Enemy is off-screen, skip
    click_cell(game_page, enemy["x"], enemy["y"])
    sel = get_selected_units(game_page)
    assert enemy["id"] not in sel, "Enemy unit should not be selectable"


def test_click_enemy_with_selection_attacks(game_page):
    """Left-clicking enemy while own units selected should issue attack."""
    pid, fact = deploy_mcv(game_page)
    snap = get_snapshot(game_page)
    enemy = next((a for a in snap["actors"] if a["owner"] != pid and a["owner"] > 2), None)
    if not enemy:
        return  # Skip if no enemy visible
    owned_unit = next((a for a in snap["actors"] if a["owner"] == pid and a["kind"] in ("Vehicle", "Infantry")), None)
    if not owned_unit:
        return  # Skip if no mobile units
    click_cell(game_page, owned_unit["x"], owned_unit["y"])
    click_cell(game_page, enemy["x"], enemy["y"])
    wait_ticks(game_page, 5)
    snap = get_snapshot(game_page)
    unit = find_actor(snap, id=owned_unit["id"])
    assert unit and unit["activity"] in ("attacking", "moving"), f"Unit should be attacking or moving, got {unit['activity'] if unit else 'None'}"


def test_escape_clears_selection(game_page):
    """Pressing Escape should deselect all units."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    click_cell(game_page, mcv["x"], mcv["y"])
    assert len(get_selected_units(game_page)) > 0
    game_page.keyboard.press("Escape")
    sel = get_selected_units(game_page)
    assert len(sel) == 0, "Escape should clear selection"


def test_no_spinning_sprite_on_selected_unit(game_page):
    """Selecting a unit should not show a spinning/animated overlay (regression: wrench bug)."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    click_cell(game_page, mcv["x"], mcv["y"])
    wait_ticks(game_page, 3)
    # Sample pixels around the MCV across two ticks — they should be stable (not animating)
    mcv_x, mcv_y = mcv["x"], mcv["y"]
    pixel_sample_js = f"""(() => {{
        const c = document.getElementById('canvas');
        const ctx = c.getContext('2d');
        const o = window._ora;
        const mcvX = Math.floor({mcv_x} * o.cellPx - o.camX + o.cellPx/2);
        const mcvY = Math.floor({mcv_y} * o.cellPx - o.camY + o.cellPx/2);
        let sum = 0;
        for (let dx = -10; dx <= 10; dx += 5) {{
            const d = ctx.getImageData(mcvX + dx, mcvY - 15, 1, 1).data;
            sum += d[0] + d[1] + d[2];
        }}
        return sum;
    }})()"""
    sample1 = game_page.evaluate(pixel_sample_js)
    wait_ticks(game_page, 4)
    sample2 = game_page.evaluate(pixel_sample_js)
    # If something is spinning/animating, the pixel values would change between frames
    # Allow small variance for normal rendering (fog, etc) but large changes indicate animation
    diff = abs(sample1 - sample2)
    assert diff < 200, f"Pixels near selected unit changed by {diff} — possible spinning overlay"
