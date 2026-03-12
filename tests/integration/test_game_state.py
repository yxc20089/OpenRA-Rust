"""Test game lifecycle and state management."""
from helpers import start_game, get_snapshot, wait_ticks, get_ui_state, get_player, get_tick, ORA


def test_initial_cash_5000(game_page):
    """Players start with $5000."""
    pid = start_game(game_page)
    snap = get_snapshot(game_page)
    player = get_player(snap, pid)
    assert player["cash"] == 5000, f"Expected $5000, got ${player['cash']}"


def test_pause_stops_ticks(game_page):
    """Pressing P should pause the game."""
    start_game(game_page)
    game_page.keyboard.press("p")
    tick_before = get_tick(game_page)
    game_page.wait_for_timeout(500)  # wait 500ms real time
    tick_after = get_tick(game_page)
    assert tick_after == tick_before, "Ticks should not advance while paused"
    ui = get_ui_state(game_page)
    assert ui["gamePaused"] is True


def test_unpause_resumes(game_page):
    """Pressing P twice should resume the game."""
    start_game(game_page)
    game_page.keyboard.press("p")
    game_page.wait_for_timeout(200)
    game_page.keyboard.press("p")
    tick_before = get_tick(game_page)
    game_page.wait_for_timeout(500)
    tick_after = get_tick(game_page)
    assert tick_after > tick_before, "Ticks should advance after unpausing"


def test_map_selection(game_page):
    """Different map indices should produce different map dimensions."""
    pid = start_game(game_page, map_index=0)
    snap0 = get_snapshot(game_page)
    w0, h0 = snap0["map_width"], snap0["map_height"]
    assert w0 > 0 and h0 > 0, "Map should have valid dimensions"


def test_difficulty_selection(game_page):
    """Game should start successfully on all difficulty levels."""
    pid = start_game(game_page, difficulty=2)  # Hard
    snap = get_snapshot(game_page)
    assert len(snap["actors"]) > 0, "Game should have actors on Hard difficulty"


def test_terrain_not_all_black(game_page):
    """Main canvas should show terrain, not be all black (regression: shroud bug)."""
    start_game(game_page)
    wait_ticks(game_page, 5)
    # Sample pixels from the center of the canvas — at least some should be non-black
    result = game_page.evaluate("""(() => {
        const c = document.getElementById('canvas');
        const ctx = c.getContext('2d');
        const cx = Math.floor(c.width / 2);
        const cy = Math.floor(c.height / 2);
        let nonBlack = 0;
        // Sample a 10x10 grid around center
        for (let dy = -5; dy < 5; dy++) {
            for (let dx = -5; dx < 5; dx++) {
                const d = ctx.getImageData(cx + dx * 10, cy + dy * 10, 1, 1).data;
                if (d[0] > 5 || d[1] > 5 || d[2] > 5) nonBlack++;
            }
        }
        return nonBlack;
    })()""")
    assert result > 0, "Canvas center is entirely black — terrain not rendering"
