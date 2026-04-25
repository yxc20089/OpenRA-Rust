//! Phase 4 basic MiniYaml fixture test.
//!
//! Parses a small (~30 line) YAML fixture that exercises:
//! - top-level nodes with children
//! - tab indentation
//! - inline `# comment` stripping
//! - empty-value block-start nodes (`Key:` with no value)
//! - named instances (`Trait@INSTANCE:`)

use openra_data::miniyaml;

const FIXTURE: &str = "\
# Top of file comment.
Title: Rush Hour Arena
Tileset: TEMPERAT
MapSize: 128,40

Players:
\tPlayerReference@Neutral:
\t\tName: Neutral
\t\tOwnsWorld: True
\t\tFaction: allies
\tPlayerReference@Multi0:
\t\tName: Multi0
\t\tPlayable: True # inline comment
\t\tFaction: Random

Actors:
\tActor0: mpspawn
\t\tOwner: Neutral
\t\tLocation: 5,6
\tActor1: e1
\t\tOwner: Multi0
\t\tLocation: 6,6
\tActor2: e1
\t\tOwner: Multi0
\t\tLocation: 7,6
";

#[test]
fn fixture_round_trip_tree_shape() {
    let nodes = miniyaml::parse(FIXTURE);

    // Top-level: Title, Tileset, MapSize, Players, Actors → 5 nodes.
    assert_eq!(
        nodes.len(),
        5,
        "expected 5 top-level nodes, got {} ({:?})",
        nodes.len(),
        nodes.iter().map(|n| &n.key).collect::<Vec<_>>()
    );
    assert_eq!(nodes[0].key, "Title");
    assert_eq!(nodes[0].value, "Rush Hour Arena");
    assert_eq!(nodes[1].key, "Tileset");
    assert_eq!(nodes[1].value, "TEMPERAT");
    assert_eq!(nodes[2].key, "MapSize");
    assert_eq!(nodes[2].value, "128,40");

    // Players block.
    let players = &nodes[3];
    assert_eq!(players.key, "Players");
    assert_eq!(players.value, "");
    assert_eq!(players.children.len(), 2);
    assert_eq!(players.children[0].key, "PlayerReference@Neutral");
    assert_eq!(players.children[0].child_value("Name"), Some("Neutral"));
    assert_eq!(players.children[0].child_value("OwnsWorld"), Some("True"));
    assert_eq!(players.children[1].key, "PlayerReference@Multi0");
    // Inline comment must be stripped.
    assert_eq!(players.children[1].child_value("Playable"), Some("True"));
    assert_eq!(players.children[1].child_value("Faction"), Some("Random"));

    // Actors block: 3 actors.
    let actors = &nodes[4];
    assert_eq!(actors.key, "Actors");
    assert_eq!(actors.children.len(), 3);
    assert_eq!(actors.children[0].key, "Actor0");
    assert_eq!(actors.children[0].value, "mpspawn");
    assert_eq!(actors.children[0].child_value("Location"), Some("5,6"));
    assert_eq!(actors.children[1].value, "e1");
    assert_eq!(actors.children[2].value, "e1");
}

#[test]
fn empty_value_node_has_no_value_but_children() {
    // `Players:` is an empty-value node with children — common for trait
    // declarations like `Mobile:` (inherit defaults, override nothing).
    let yaml = "Mobile:\n\tSpeed: 54\n";
    let nodes = miniyaml::parse(yaml);
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].key, "Mobile");
    assert_eq!(nodes[0].value, "");
    assert_eq!(nodes[0].children.len(), 1);
    assert_eq!(nodes[0].children[0].key, "Speed");
    assert_eq!(nodes[0].children[0].value, "54");
}
