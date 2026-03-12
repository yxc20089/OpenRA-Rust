"""Shared utilities for OpenRA integration tests."""
import json

# All game state is accessed via window._ora (exposed from ES module)
ORA = "window._ora"


# -- Game lifecycle --

def start_game(page, map_index=0, difficulty=0):
    """Start a new game with given map and difficulty. Returns humanPlayerId."""
    page.select_option("#map-select", str(map_index))
    page.select_option("#difficulty-select", str(difficulty))
    page.click("#btn-start-game")
    page.wait_for_selector("#game-ui", state="visible", timeout=15000)
    page.wait_for_function(f"{ORA}.currentTick > 0", timeout=15000)
    return page.evaluate(f"{ORA}.humanPlayerId")


def get_snapshot(page):
    """Get current game world snapshot as dict."""
    return page.evaluate(f"JSON.parse({ORA}.session.snapshot_json())")


def wait_ticks(page, n, timeout_ms=None):
    """Wait for N game ticks to elapse."""
    if timeout_ms is None:
        timeout_ms = max(n * 200, 10000)
    start = page.evaluate(f"{ORA}.currentTick")
    page.wait_for_function(f"{ORA}.currentTick >= {start + n}", timeout=timeout_ms)


def get_tick(page):
    """Get the current game tick."""
    return page.evaluate(f"{ORA}.currentTick")


# -- UI state queries --

def get_ui_state(page):
    """Get UI-level state (selection, placement, command mode, etc.)."""
    return page.evaluate(f"""(function() {{
        const o = {ORA};
        return {{
            selectedUnits: o.selectedUnits,
            placementMode: o.placementMode,
            commandMode: o.commandMode,
            controlGroups: o.controlGroups,
            gamePaused: o.gamePaused,
            exploredCells: o.exploredCells,
            mode: o.mode,
            camX: o.camX,
            camY: o.camY,
            cellPx: o.cellPx,
        }};
    }})()""")


def get_selected_units(page):
    """Get list of currently selected unit IDs."""
    return page.evaluate(f"{ORA}.selectedUnits")


def get_cam(page):
    """Get camera position and zoom."""
    return page.evaluate(f"({{ camX: {ORA}.camX, camY: {ORA}.camY, cellPx: {ORA}.cellPx }})")


# -- Canvas interaction --

def click_cell(page, cell_x, cell_y, button="left"):
    """Click a game world cell on the canvas.

    If the target cell is outside the current viewport (e.g. due to edge-scroll
    drift), the camera is automatically re-centered on the cell before clicking.
    """
    cam = get_cam(page)
    canvas = page.locator("#canvas")
    box = canvas.bounding_box()
    px = (cell_x * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
    py = (cell_y * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    if not (0 <= px <= box["width"] and 0 <= py <= box["height"]):
        # Re-center camera on the target cell so it's in the viewport
        page.evaluate(
            f"{ORA}.setCam("
            f"{cell_x} * {ORA}.cellPx - document.getElementById('canvas').width / 2, "
            f"{cell_y} * {ORA}.cellPx - document.getElementById('canvas').height / 2)"
        )
        cam = get_cam(page)
        px = (cell_x * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
        py = (cell_y * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    canvas.click(position={"x": px, "y": py}, button=button)


def right_click_cell(page, cell_x, cell_y):
    """Right-click a game world cell."""
    click_cell(page, cell_x, cell_y, button="right")


def shift_click_cell(page, cell_x, cell_y):
    """Shift+left-click a game world cell."""
    cam = get_cam(page)
    canvas = page.locator("#canvas")
    box = canvas.bounding_box()
    px = (cell_x * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
    py = (cell_y * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    if not (0 <= px <= box["width"] and 0 <= py <= box["height"]):
        # Re-center camera on the target cell
        page.evaluate(
            f"{ORA}.setCam("
            f"{cell_x} * {ORA}.cellPx - document.getElementById('canvas').width / 2, "
            f"{cell_y} * {ORA}.cellPx - document.getElementById('canvas').height / 2)"
        )
        cam = get_cam(page)
        px = (cell_x * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
        py = (cell_y * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    page.keyboard.down("Shift")
    canvas.click(position={"x": px, "y": py})
    page.keyboard.up("Shift")


def drag_select(page, x1, y1, x2, y2):
    """Drag-select from cell (x1,y1) to cell (x2,y2)."""
    cam = get_cam(page)
    canvas = page.locator("#canvas")
    box = canvas.bounding_box()
    px1 = (x1 * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
    py1 = (y1 * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    px2 = (x2 * cam["cellPx"]) - cam["camX"] + cam["cellPx"] // 2
    py2 = (y2 * cam["cellPx"]) - cam["camY"] + cam["cellPx"] // 2
    # Convert to page coordinates
    abs_x1 = box["x"] + px1
    abs_y1 = box["y"] + py1
    abs_x2 = box["x"] + px2
    abs_y2 = box["y"] + py2
    page.mouse.move(abs_x1, abs_y1)
    page.mouse.down()
    page.mouse.move(abs_x2, abs_y2)
    page.mouse.up()


# -- Orders (via WASM session) --

def order_move(page, unit_id, x, y):
    page.evaluate(f"{ORA}.session.order_move({unit_id}, {x}, {y})")

def order_attack(page, unit_id, target_id):
    page.evaluate(f"{ORA}.session.order_attack({unit_id}, {target_id})")

def order_attack_move(page, unit_id, x, y):
    page.evaluate(f"{ORA}.session.order_attack_move({unit_id}, {x}, {y})")

def order_stop(page, unit_id):
    page.evaluate(f"{ORA}.session.order_stop({unit_id})")

def order_deploy(page, unit_id):
    page.evaluate(f"{ORA}.session.order_deploy({unit_id})")

def order_start_production(page, item_name):
    page.evaluate(f"{ORA}.session.order_start_production('{item_name}')")

def order_place_building(page, building_type, x, y):
    page.evaluate(f"{ORA}.session.order_place_building('{building_type}', {x}, {y})")

def can_place_building(page, building_type, x, y):
    return page.evaluate(f"{ORA}.session.can_place_building('{building_type}', {x}, {y})")

def order_sell(page, building_id):
    page.evaluate(f"{ORA}.session.order_sell({building_id})")

def order_repair(page, building_id):
    page.evaluate(f"{ORA}.session.order_repair({building_id})")

def order_set_rally_point(page, building_id, x, y):
    page.evaluate(f"{ORA}.session.order_set_rally_point({building_id}, {x}, {y})")


# -- Snapshot queries --

def find_actor(snapshot, **kwargs):
    """Find first actor matching all kwargs."""
    for a in snapshot["actors"]:
        if all(a.get(k) == v for k, v in kwargs.items()):
            return a
    return None


def find_actors(snapshot, **kwargs):
    """Find all actors matching kwargs."""
    return [a for a in snapshot["actors"] if all(a.get(k) == v for k, v in kwargs.items())]


def get_player(snapshot, pid):
    """Get player snapshot by ID."""
    return next((p for p in snapshot["players"] if p["index"] == pid), None)


# -- Common setup helpers --

def deploy_mcv(page):
    """Deploy MCV and return (pid, fact_actor). Assumes game already started."""
    pid = start_game(page)
    snap = get_snapshot(page)
    mcv = find_actor(snap, kind="Mcv", owner=pid)
    assert mcv, "No MCV found"
    order_deploy(page, mcv["id"])
    wait_ticks(page, 40)
    snap = get_snapshot(page)
    fact = find_actor(snap, actor_type="fact", owner=pid)
    assert fact, "FACT not created"
    return pid, fact


def build_and_place(page, pid, building_type, fact, offset_x=3, offset_y=0):
    """Produce and place a building adjacent to fact. Returns placed actor."""
    order_start_production(page, building_type)
    wait_ticks(page, 350)
    place_x = fact["x"] + offset_x
    place_y = fact["y"] + offset_y
    order_place_building(page, building_type, place_x, place_y)
    wait_ticks(page, 2)
    snap = get_snapshot(page)
    return find_actor(snap, actor_type=building_type, owner=pid)


def deploy_and_build_base(page):
    """Full base setup: deploy MCV, powr, tent, weap. Returns (pid, fact, tent, weap)."""
    pid, fact = deploy_mcv(page)
    powr = build_and_place(page, pid, "powr", fact, offset_x=3, offset_y=0)
    tent = build_and_place(page, pid, "tent", fact, offset_x=-2, offset_y=0)
    weap = build_and_place(page, pid, "weap", fact, offset_x=0, offset_y=2)
    return pid, fact, tent, weap
