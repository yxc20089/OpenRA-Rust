//! Phase 4 inheritance fixture test.
//!
//! Verifies that `Inherits: ^Parent` correctly merges parent children into
//! the child, that overrides win, and that `-Trait:` removes inherited
//! traits. This mirrors the pattern used by `^Infantry → ^Soldier → E1`
//! in `vendor/OpenRA/mods/ra/rules`.

use openra_data::miniyaml;

const FIXTURE: &str = "\
# Abstract parent (will not appear in resolved output).
^Infantry:
\tHealth:
\t\tHP: 2500
\tMobile:
\t\tSpeed: 54
\tRevealsShroud:
\t\tRange: 4c0
\tArmor:
\t\tType: None

^Soldier:
\tInherits: ^Infantry
\tMustBeDestroyed:
\tArmament:
\t\tWeapon: BasicGun

# Concrete actor: inherits from ^Soldier (which itself inherits ^Infantry),
# overrides HP, removes the Armor trait, adds a fresh Buildable trait.
Rifleman:
\tInherits: ^Soldier
\tHealth:
\t\tHP: 5000
\t-Armor:
\tBuildable:
\t\tQueue: Infantry
";

#[test]
fn resolves_two_level_inheritance_chain() {
    let nodes = miniyaml::parse(FIXTURE);
    let resolved = miniyaml::resolve_inherits(nodes);

    // Abstract parents `^Infantry` and `^Soldier` must be stripped.
    let keys: Vec<&str> = resolved.iter().map(|n| n.key.as_str()).collect();
    assert_eq!(
        keys,
        vec!["Rifleman"],
        "abstract parents leaked into resolved output: {:?}",
        keys
    );

    let r = &resolved[0];

    // Inherited traits flow through both levels.
    assert!(r.child("Mobile").is_some(), "Mobile should inherit from ^Infantry");
    assert_eq!(
        r.child("Mobile").unwrap().child_value("Speed"),
        Some("54"),
        "Mobile.Speed should pass through unchanged"
    );
    assert!(r.child("RevealsShroud").is_some(), "RevealsShroud inherits");
    assert!(r.child("MustBeDestroyed").is_some(), "MustBeDestroyed inherits from ^Soldier");
    assert!(r.child("Armament").is_some(), "Armament inherits from ^Soldier");
    assert_eq!(
        r.child("Armament").unwrap().child_value("Weapon"),
        Some("BasicGun")
    );

    // Override wins over the inherited value.
    assert_eq!(
        r.child("Health").unwrap().child_value("HP"),
        Some("5000"),
        "Rifleman should override ^Infantry's HP=2500 with HP=5000"
    );

    // Removal directive strips the inherited Armor trait.
    assert!(r.child("Armor").is_none(), "-Armor: should remove inherited Armor trait");

    // The child's own new trait survives.
    assert!(r.child("Buildable").is_some());
    assert_eq!(r.child("Buildable").unwrap().child_value("Queue"), Some("Infantry"));

    // Inherits: line itself must NOT appear as a child of Rifleman.
    assert!(
        !r.children.iter().any(|c| c.key == "Inherits"),
        "Inherits: directive should be consumed during resolution"
    );
}

#[test]
fn override_only_replaces_explicitly_set_subkeys() {
    // Verifies the merge behaviour: when the child redefines a trait block
    // partially, sibling keys in the parent's block are preserved.
    let yaml = "\
^Base:
\tMobile:
\t\tSpeed: 100
\t\tLocomotor: foot

Child:
\tInherits: ^Base
\tMobile:
\t\tSpeed: 200
";
    let nodes = miniyaml::parse(yaml);
    let resolved = miniyaml::resolve_inherits(nodes);
    let mobile = resolved[0].child("Mobile").unwrap();
    assert_eq!(mobile.child_value("Speed"), Some("200"), "speed override applies");
    assert_eq!(
        mobile.child_value("Locomotor"),
        Some("foot"),
        "untouched parent key should pass through the merge"
    );
}
