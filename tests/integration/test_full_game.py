"""Comprehensive scripted game test: MCV → base → army → combat.

Takes screenshots at every step so rendering issues are visible.
Tests facing, movement, production, placement, and combat end-to-end.
"""
import os
import json
from helpers import (
    start_game, get_snapshot, wait_ticks, click_cell, right_click_cell,
    find_actor, find_actors, get_selected_units, deploy_mcv,
    order_move, order_deploy, order_stop, order_attack,
    order_start_production, order_place_building, can_place_building,
    order_sell, get_cam, get_tick, get_player, get_ui_state,
    build_and_place, ORA,
)

SCREENSHOT_DIR = os.path.join(os.path.dirname(__file__), "screenshots")
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

_step = 0

def screenshot(page, name):
    global _step
    _step += 1
    path = os.path.join(SCREENSHOT_DIR, f"{_step:03d}_{name}.png")
    page.screenshot(path=path)
    return path


def test_full_game_session(game_page):
    """Full scripted game: deploy, build, produce, move, attack — with screenshots."""
    global _step
    _step = 0
    page = game_page

    # ── 1. Start game ──
    pid = start_game(page)
    snap = get_snapshot(page)
    screenshot(page, "01_game_start")

    mcv = find_actor(snap, kind="Mcv", owner=pid)
    assert mcv, "MCV not found at game start"
    mcv_x, mcv_y = mcv["x"], mcv["y"]

    # ── 2. Verify initial state ──
    player = get_player(snap, pid)
    assert player["cash"] == 5000, f"Starting cash should be 5000, got {player['cash']}"

    # ── 3. Select MCV ──
    click_cell(page, mcv_x, mcv_y)
    sel = get_selected_units(page)
    assert mcv["id"] in sel, f"MCV should be selected, got {sel}"
    screenshot(page, "02_mcv_selected")

    # ── 4. Move MCV east (test facing) ──
    target_x = mcv_x + 3
    order_move(page, mcv["id"], target_x, mcv_y)
    wait_ticks(page, 5)
    screenshot(page, "03_mcv_moving_east")

    snap = get_snapshot(page)
    moving_mcv = find_actor(snap, id=mcv["id"])
    # After a few ticks, facing should be East (768 in CCW convention)
    # Allow for turning time
    print(f"MCV facing after move east: {moving_mcv['facing']}, activity: {moving_mcv['activity']}")

    wait_ticks(page, 30)
    snap = get_snapshot(page)
    moving_mcv = find_actor(snap, id=mcv["id"])
    screenshot(page, "04_mcv_moved_east")
    print(f"MCV position after move: ({moving_mcv['x']},{moving_mcv['y']}), facing: {moving_mcv['facing']}")

    # ── 5. Move MCV diagonally SE (the reported bug) ──
    order_move(page, mcv["id"], moving_mcv["x"] + 3, moving_mcv["y"] + 3)
    wait_ticks(page, 5)
    screenshot(page, "05_mcv_moving_SE")
    snap = get_snapshot(page)
    se_mcv = find_actor(snap, id=mcv["id"])
    print(f"MCV facing during SE move: {se_mcv['facing']}, activity: {se_mcv['activity']}")

    wait_ticks(page, 30)
    screenshot(page, "06_mcv_after_SE_move")

    # ── 6. Move MCV back and deploy ──
    snap = get_snapshot(page)
    mcv_now = find_actor(snap, id=mcv["id"])
    # Stop and deploy
    order_stop(page, mcv["id"])
    wait_ticks(page, 2)
    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)
    screenshot(page, "07_mcv_deployed")

    snap = get_snapshot(page)
    fact = find_actor(snap, actor_type="fact", owner=pid)
    assert fact, "Construction Yard not created after deploy"
    print(f"FACT at ({fact['x']},{fact['y']})")

    # ── 7. Build power plant ──
    order_start_production(page, "powr")
    wait_ticks(page, 350)
    screenshot(page, "08_powr_ready")

    # Place power plant east of fact
    px, py = fact["x"] + 3, fact["y"]
    placed = can_place_building(page, "powr", px, py)
    print(f"Can place powr at ({px},{py}): {placed}")
    order_place_building(page, "powr", px, py)
    wait_ticks(page, 5)
    screenshot(page, "09_powr_placed")

    snap = get_snapshot(page)
    powr = find_actor(snap, actor_type="powr", owner=pid)
    assert powr, "Power plant not placed"

    # Check power
    player = get_player(snap, pid)
    print(f"Power after powr: provided={player.get('power_provided',0)}, drained={player.get('power_drained',0)}")

    # ── 8. Build refinery (required for weap) ──
    order_start_production(page, "proc")
    wait_ticks(page, 350)
    rx, ry = fact["x"] - 3, fact["y"]
    order_place_building(page, "proc", rx, ry)
    wait_ticks(page, 5)

    snap = get_snapshot(page)
    proc = find_actor(snap, actor_type="proc", owner=pid)
    assert proc, "Refinery not placed"

    # ── 9. Build barracks ──
    order_start_production(page, "tent")
    wait_ticks(page, 350)
    tx, ty = fact["x"] - 2, fact["y"] + 3
    order_place_building(page, "tent", tx, ty)
    wait_ticks(page, 5)
    screenshot(page, "10_tent_placed")

    snap = get_snapshot(page)
    tent = find_actor(snap, actor_type="tent", owner=pid)
    assert tent, "Barracks not placed"

    # ── 10. Build war factory ──
    order_start_production(page, "weap")
    wait_ticks(page, 400)
    wx, wy = fact["x"], fact["y"] + 3
    order_place_building(page, "weap", wx, wy)
    wait_ticks(page, 5)
    screenshot(page, "11_weap_placed")

    snap = get_snapshot(page)
    weap = find_actor(snap, actor_type="weap", owner=pid)
    assert weap, "War Factory not placed"

    # ── 10. Train infantry ──
    order_start_production(page, "e1")
    wait_ticks(page, 200)
    screenshot(page, "12_infantry_produced")

    snap = get_snapshot(page)
    infantry = find_actors(snap, actor_type="e1", owner=pid)
    print(f"Infantry produced: {len(infantry)}")

    # ── 11. Train tank ──
    order_start_production(page, "1tnk")
    wait_ticks(page, 300)
    screenshot(page, "13_tank_produced")

    snap = get_snapshot(page)
    tanks = find_actors(snap, actor_type="1tnk", owner=pid)
    print(f"Tanks produced: {len(tanks)}")

    # ── 12. Select and move units ──
    snap = get_snapshot(page)
    my_units = [a for a in snap["actors"] if a["owner"] == pid
                and a["kind"] in ("Vehicle", "Infantry")]
    if my_units:
        unit = my_units[0]
        click_cell(page, unit["x"], unit["y"])
        screenshot(page, "14_unit_selected")

        # Move unit north
        order_move(page, unit["id"], unit["x"], unit["y"] - 5)
        wait_ticks(page, 5)
        screenshot(page, "15_unit_moving_north")
        snap = get_snapshot(page)
        moved = find_actor(snap, id=unit["id"])
        print(f"Unit {unit['actor_type']} facing during N move: {moved['facing']}, activity: {moved['activity']}")

        # Facing check: North = 0
        wait_ticks(page, 10)
        snap = get_snapshot(page)
        moved = find_actor(snap, id=unit["id"])
        if moved:
            facing_diff = min(abs(moved["facing"] - 0), 1024 - abs(moved["facing"] - 0))
            print(f"Unit facing after N move: {moved['facing']} (diff from 0: {facing_diff})")
        else:
            print(f"Unit {unit['id']} no longer exists (killed?), skipping facing checks")

        # Move unit SE (diagonal — the reported bug)
        if moved:
            order_move(page, unit["id"], unit["x"] + 5, unit["y"] + 5)
            wait_ticks(page, 3)
            screenshot(page, "16_unit_moving_SE")
            snap = get_snapshot(page)
            moved = find_actor(snap, id=unit["id"])
            if moved:
                print(f"Unit facing during SE move: {moved['facing']}")
            # SE should be facing 640 in CCW convention
            wait_ticks(page, 10)
        snap = get_snapshot(page)
        moved = find_actor(snap, id=unit["id"])
        if moved:
            facing_diff_se = min(abs(moved["facing"] - 640), 1024 - abs(moved["facing"] - 640))
            print(f"Unit facing after SE move: {moved['facing']} (diff from 640: {facing_diff_se})")
        screenshot(page, "17_unit_after_SE_move")

        # Move unit NW (opposite diagonal)
        if moved:
            order_move(page, unit["id"], unit["x"] - 3, unit["y"] - 3)
            wait_ticks(page, 10)
            screenshot(page, "18_unit_moving_NW")
            snap = get_snapshot(page)
            moved = find_actor(snap, id=unit["id"])
            if moved:
                print(f"Unit facing during NW move: {moved['facing']}")
                facing_diff_nw = min(abs(moved["facing"] - 128), 1024 - abs(moved["facing"] - 128))
                print(f"Unit facing after NW move: {moved['facing']} (diff from 128: {facing_diff_nw})")

    # ── 13. Check for rendering issues: mines, trees, resources ──
    snap = get_snapshot(page)
    mines = find_actors(snap, kind="Mine")
    trees = find_actors(snap, kind="Tree")
    print(f"Map has {len(mines)} mines, {len(trees)} trees")
    for m in mines[:3]:
        print(f"  Mine: type={m['actor_type']}, pos=({m['x']},{m['y']})")

    # Pan to ore field if there are mines
    if mines:
        mine = mines[0]
        page.evaluate(f"""(() => {{
            const o = window._ora;
            const c = document.getElementById('canvas');
            o.setCam({mine['x']} * o.cellPx - c.width / 2,
                     {mine['y']} * o.cellPx - c.height / 2);
        }})()""")
        # Use keyboard to pan to mine area
        cam = get_cam(page)
        # Just take screenshot of current view which should have ore nearby
        wait_ticks(page, 2)
        screenshot(page, "19_ore_area")

    # ── 14. Check resource overlay ──
    has_resources = page.evaluate(f"""(() => {{
        const snap = JSON.parse({ORA}.session.snapshot_json());
        return snap.resources ? snap.resources.length : 0;
    }})()""")
    print(f"Resource tiles: {has_resources}")

    # ── 15. Find enemy and attack ──
    snap = get_snapshot(page)
    enemies = [a for a in snap["actors"] if a["owner"] != pid and a["owner"] > 2
               and a["kind"] in ("Vehicle", "Infantry", "Building", "Mcv")]
    print(f"Visible enemies: {len(enemies)}")

    if my_units and enemies:
        unit = my_units[0]
        enemy = enemies[0]
        order_attack(page, unit["id"], enemy["id"])
        wait_ticks(page, 5)
        screenshot(page, "20_attacking_enemy")
        snap = get_snapshot(page)
        attacker = find_actor(snap, id=unit["id"])
        print(f"Attacker activity: {attacker['activity'] if attacker else 'dead'}")

    # ── 16. Sell a building ──
    snap = get_snapshot(page)
    powr = find_actor(snap, actor_type="powr", owner=pid)
    if powr:
        cash_before = get_player(snap, pid)["cash"]
        order_sell(page, powr["id"])
        wait_ticks(page, 5)
        screenshot(page, "21_building_sold")
        snap = get_snapshot(page)
        cash_after = get_player(snap, pid)["cash"]
        print(f"Cash before sell: {cash_before}, after: {cash_after}")
        assert cash_after > cash_before, "Selling should refund cash"

    # ── 17. Final state screenshot ──
    wait_ticks(page, 10)
    screenshot(page, "22_final_state")

    # ── Summary ──
    snap = get_snapshot(page)
    my_actors = [a for a in snap["actors"] if a["owner"] == pid]
    buildings = [a for a in my_actors if a["kind"] == "Building"]
    units = [a for a in my_actors if a["kind"] in ("Vehicle", "Infantry")]
    print(f"\n=== FINAL STATE ===")
    print(f"Buildings: {len(buildings)} ({[b['actor_type'] for b in buildings]})")
    print(f"Units: {len(units)} ({[u['actor_type'] for u in units]})")
    print(f"Cash: {get_player(snap, pid)['cash']}")
    print(f"Tick: {get_tick(page)}")
    print(f"Screenshots saved to: {SCREENSHOT_DIR}")
