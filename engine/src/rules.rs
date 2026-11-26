//! Multi-scale ownership & permission Rule Book (Phase A).
//!
//! Enforced strictly by the engine / authoritative server (never trusted to `.wasm` modules).
//! Permissions operate across four dimensions:
//! - **Principals**: area `owners`, `builders`, `visitors`, plus each object's owner (the world
//!   graph's `owned_by`, `graph`).
//! - **Spatial scopes**: 64 m tile policies (`AreaPolicy`), optional claimed sub-volumes (`Plot`,
//!   e.g. a tiny home plot inside a shared forest), and protected `border_guard` / `spawn_guard`
//!   buffers for non-owners.
//! - **Scale caps**: orthogonal `sculpt_max`, `place_max`, and `water_max` (`None` or `Some(Size)`),
//!   allowing e.g. tiny-only building inside a shared area while blocking giant terraforming.
//! - **Object ownership**: `CutCarryPolicy::{None, OwnOnly, Any}` so visitors can relocate or cut
//!   their own placed objects without griefing others'.

use wasmtime::bail;

use crate::{Result, START, Size, WIDTH};

/// Default player identifier in local single-player sessions.
pub const LOCAL_PLAYER: &str = "local";

/// Effective role a player holds on an area tile or plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Owner,
    Builder,
    Visitor,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Builder => "builder",
            Role::Visitor => "visitor",
        }
    }
}

/// Who may chop (`K`), grab/carry (`G`), or remove placed procedural objects.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CutCarryPolicy {
    None,
    OwnOnly,
    Any,
}

impl CutCarryPolicy {
    pub fn name(self) -> &'static str {
        match self {
            CutCarryPolicy::None => "none",
            CutCarryPolicy::OwnOnly => "own",
            CutCarryPolicy::Any => "any",
        }
    }

    pub fn from_name(s: &str) -> Option<CutCarryPolicy> {
        match s {
            "none" => Some(CutCarryPolicy::None),
            "own" => Some(CutCarryPolicy::OwnOnly),
            "any" => Some(CutCarryPolicy::Any),
            _ => None,
        }
    }
}

/// Capabilities granted to a role (builder or visitor) on an area tile or plot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RolePolicy {
    pub visit: bool,
    pub interact: bool,
    /// Maximum player size allowed to sculpt/carve terrain (`None` = forbidden).
    pub sculpt_max: Option<Size>,
    /// Maximum player size allowed to place standalone `.wasm` elements (`None` = forbidden).
    pub place_max: Option<Size>,
    /// Maximum player size allowed to modify/expand/add water (`None` = forbidden).
    pub water_max: Option<Size>,
    /// Whether chopping, carrying, or removing placed objects is allowed.
    pub cut_carry: CutCarryPolicy,
}

impl RolePolicy {
    /// Full permissions at every scale (owners & unclaimed single-player areas).
    pub fn full() -> RolePolicy {
        RolePolicy {
            visit: true,
            interact: true,
            sculpt_max: Some(Size::Giant),
            place_max: Some(Size::Giant),
            water_max: Some(Size::Giant),
            cut_carry: CutCarryPolicy::Any,
        }
    }

    /// Standard trusted-builder permissions (up to person scale; no giant-scale destruction).
    pub fn builder() -> RolePolicy {
        RolePolicy {
            visit: true,
            interact: true,
            sculpt_max: Some(Size::Person),
            place_max: Some(Size::Person),
            water_max: Some(Size::Person),
            cut_carry: CutCarryPolicy::Any,
        }
    }

    /// Tiny-friendly community visitor policy: visitors may carve/furnish tiny homes and carry
    /// their own placed items without altering person/giant landmarks or water.
    pub fn tiny_visitor() -> RolePolicy {
        RolePolicy {
            visit: true,
            interact: true,
            sculpt_max: Some(Size::Tiny),
            place_max: Some(Size::Tiny),
            water_max: None,
            cut_carry: CutCarryPolicy::OwnOnly,
        }
    }

    /// Read-only visitor: can walk and interact with doors/lanterns, but cannot alter the area.
    pub fn read_only() -> RolePolicy {
        RolePolicy {
            visit: true,
            interact: true,
            sculpt_max: None,
            place_max: None,
            water_max: None,
            cut_carry: CutCarryPolicy::None,
        }
    }

    /// Encodes as compact key/value string for `WorldSave` (`visit/interact/sculpt/place/water/cut`).
    pub fn encode(&self) -> String {
        let sz = |o: Option<Size>| o.map_or("none", Size::name);
        format!(
            "{},{},{},{},{},{}",
            self.visit as u8,
            self.interact as u8,
            sz(self.sculpt_max),
            sz(self.place_max),
            sz(self.water_max),
            self.cut_carry.name()
        )
    }

    pub fn decode(s: &str) -> Result<RolePolicy> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 6 {
            bail!("expected 6 comma-separated RolePolicy fields, got `{s}`");
        }
        let pb = |i: usize| -> Result<bool> {
            match parts[i] {
                "0" => Ok(false),
                "1" => Ok(true),
                other => bail!("invalid bool `{other}` in RolePolicy"),
            }
        };
        let ps = |i: usize| -> Result<Option<Size>> {
            match parts[i] {
                "none" => Ok(None),
                "tiny" => Ok(Some(Size::Tiny)),
                "person" => Ok(Some(Size::Person)),
                "giant" => Ok(Some(Size::Giant)),
                other => bail!("invalid size `{other}` in RolePolicy"),
            }
        };
        let cut_carry = CutCarryPolicy::from_name(parts[5]).ok_or_else(|| wasmtime::format_err!("invalid cut_carry `{}`", parts[5]))?;
        Ok(RolePolicy {
            visit: pb(0)?,
            interact: pb(1)?,
            sculpt_max: ps(2)?,
            place_max: ps(3)?,
            water_max: ps(4)?,
            cut_carry,
        })
    }
}

/// A claimed 3D sub-volume inside the world (e.g. one tree trunk cavity leased to a tiny builder
/// inside a shared forest tile).
#[derive(Clone, Debug, PartialEq)]
pub struct Plot {
    pub id: u32,
    pub name: String,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub owners: Vec<String>,
    pub builders: Vec<String>,
    pub builder_policy: RolePolicy,
    pub visitor_policy: Option<RolePolicy>,
}

impl Plot {
    pub fn contains_point(&self, p: [f32; 3]) -> bool {
        (0..3).all(|a| p[a] >= self.lo[a] && p[a] <= self.hi[a])
    }

    pub fn contains_box(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        (0..3).all(|a| lo[a] >= self.lo[a] && hi[a] <= self.hi[a])
    }

    pub fn overlaps_box(&self, lo: [f32; 3], hi: [f32; 3]) -> bool {
        (0..3).all(|a| hi[a] >= self.lo[a] && lo[a] <= self.hi[a])
    }

    pub fn validate(&self) -> Result<()> {
        if self.id == 0 {
            bail!("plot id must be >= 1");
        }
        if self.name.is_empty() || self.name.chars().any(char::is_whitespace) {
            bail!("plot name must be non-empty without whitespace, got `{}`", self.name);
        }
        for a in 0..3 {
            if !self.lo[a].is_finite() || !self.hi[a].is_finite() || self.lo[a] >= self.hi[a] {
                bail!("plot `{}` has invalid bounds lo={:?} hi={:?}", self.name, self.lo, self.hi);
            }
        }
        validate_principals(&self.owners, "plot owners")?;
        validate_principals(&self.builders, "plot builders")?;
        Ok(())
    }
}

/// Ownership and permission rules for one 64 m × 64 m tile `[gx, gz]`.
///
/// When `owners` is empty, the tile is treated as **unclaimed / open** (full permissions for local
/// single-player play and community sandboxes). Once at least one owner is listed, non-owners are
/// governed strictly by `builder_policy` / `visitor_policy`, `border_guard`, and `spawn_guard`.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaPolicy {
    pub tile: [i32; 2],
    pub owners: Vec<String>,
    pub builders: Vec<String>,
    pub builder_policy: RolePolicy,
    pub visitor_policy: RolePolicy,
    /// Non-owners cannot sculpt terrain within this many metres of any tile edge (`0.0` disables).
    pub border_guard: f32,
    /// Non-owners cannot sculpt terrain within this radius of the tile's spawn point `(32, 60)`.
    pub spawn_guard: f32,
}

impl AreaPolicy {
    /// Open unclaimed policy (default for single-player worlds).
    pub fn open(tile: [i32; 2]) -> AreaPolicy {
        AreaPolicy {
            tile,
            owners: Vec::new(),
            builders: Vec::new(),
            builder_policy: RolePolicy::full(),
            visitor_policy: RolePolicy::full(),
            border_guard: 0.0,
            spawn_guard: 0.0,
        }
    }

    /// Standard owned tile: full rights for `owner`, `builder()` for builders, `read_only()` for
    /// visitors, with `2.0 m` border guard and `3.0 m` spawn guard.
    pub fn owned(tile: [i32; 2], owner: impl Into<String>) -> AreaPolicy {
        AreaPolicy {
            tile,
            owners: vec![owner.into()],
            builders: Vec::new(),
            builder_policy: RolePolicy::builder(),
            visitor_policy: RolePolicy::read_only(),
            border_guard: 2.0,
            spawn_guard: 3.0,
        }
    }

    pub fn is_claimed(&self) -> bool {
        !self.owners.is_empty()
    }

    pub fn role_of(&self, player: &str) -> Role {
        if !self.is_claimed() || self.owners.iter().any(|o| o == player) {
            Role::Owner
        } else if self.builders.iter().any(|b| b == player) {
            Role::Builder
        } else {
            Role::Visitor
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_principals(&self.owners, "area owners")?;
        validate_principals(&self.builders, "area builders")?;
        if !self.border_guard.is_finite() || self.border_guard < 0.0 || self.border_guard > WIDTH * 0.5 {
            bail!("invalid border_guard {}", self.border_guard);
        }
        if !self.spawn_guard.is_finite() || self.spawn_guard < 0.0 || self.spawn_guard > WIDTH {
            bail!("invalid spawn_guard {}", self.spawn_guard);
        }
        Ok(())
    }
}

/// One player action validated against the `RuleBook`.
#[derive(Clone, Debug)]
pub enum Action<'a> {
    Visit { at: [f32; 3] },
    Interact { at: [f32; 3] },
    Sculpt { lo: [f32; 3], hi: [f32; 3], player_size: f32, is_water: bool },
    PlaceElement { at: [f32; 3], player_size: f32 },
    /// `owner`: who owns the element (the world graph's `owned_by`).
    RemoveElement { at: [f32; 3], owner: Option<&'a str> },
    CutOrCarry { at: [f32; 3], owner: Option<&'a str> },
    SculptElement { at: [f32; 3], player_size: f32, owner: Option<&'a str> },
    ModifyWater { lo: [f32; 3], hi: [f32; 3], player_size: f32 },
}

impl Action<'_> {
    fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        match *self {
            Action::Visit { at }
            | Action::Interact { at }
            | Action::PlaceElement { at, .. }
            | Action::RemoveElement { at, .. }
            | Action::CutOrCarry { at, .. }
            | Action::SculptElement { at, .. } => (at, at),
            Action::Sculpt { lo, hi, .. } | Action::ModifyWater { lo, hi, .. } => (lo, hi),
        }
    }
}

/// Explanation returned when the `RuleBook` blocks an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Denial {
    pub tile: [i32; 2],
    pub reason: String,
}

impl std::fmt::Display for Denial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

/// Complete multi-area permission table persisted alongside a world manifest.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RuleBook {
    pub areas: Vec<AreaPolicy>,
    pub plots: Vec<Plot>,
}

impl RuleBook {
    pub fn new() -> RuleBook {
        RuleBook::default()
    }

    pub fn is_empty(&self) -> bool {
        self.areas.is_empty() && self.plots.is_empty()
    }

    /// Inserts or replaces the `AreaPolicy` for `policy.tile`.
    pub fn set_area(&mut self, policy: AreaPolicy) {
        if let Some(existing) = self.areas.iter_mut().find(|a| a.tile == policy.tile) {
            *existing = policy;
        } else {
            self.areas.push(policy);
        }
    }

    /// Inserts or replaces a `Plot` by `plot.id`.
    pub fn set_plot(&mut self, plot: Plot) {
        if let Some(existing) = self.plots.iter_mut().find(|p| p.id == plot.id) {
            *existing = plot;
        } else {
            self.plots.push(plot);
        }
    }

    pub fn area_policy(&self, tile: [i32; 2]) -> Option<&AreaPolicy> {
        self.areas.iter().find(|a| a.tile == tile)
    }

    /// Checks whether `player` is allowed to perform `action`.
    /// Validates every tile overlapped by the action's bounding box, honoring sub-plot grants
    /// when an action lies strictly inside a plot where the player has elevated rights, and
    /// protecting claimed plots from outsiders.
    pub fn check(&self, player: &str, action: &Action<'_>) -> std::result::Result<(), Denial> {
        let (lo, hi) = action.bounds();

        // 1. Check any overlapping plots first:
        //    - If the action lies entirely inside a plot where `player` is an owner/builder,
        //      grant that plot's role (allowing a tiny builder to furnish their own cavity plot
        //      inside someone else's forest!).
        //    - If the action overlaps a plot owned by someone else where `player` is NOT an
        //      owner/builder, enforce that plot's visitor_policy (if set) or deny destructive edits.
        let mut granted_by_plot: Option<(Role, RolePolicy)> = None;
        for plot in &self.plots {
            if !plot.overlaps_box(lo, hi) {
                continue;
            }
            let is_plot_owner = plot.owners.iter().any(|o| o == player);
            let is_plot_builder = plot.builders.iter().any(|b| b == player);
            if plot.contains_box(lo, hi) && (is_plot_owner || is_plot_builder) {
                granted_by_plot = Some(if is_plot_owner {
                    (Role::Owner, RolePolicy::full())
                } else {
                    (Role::Builder, plot.builder_policy)
                });
                continue;
            }
            // Action touches a plot owned by someone else.
            if !plot.owners.is_empty() && !is_plot_owner && !is_plot_builder {
                let tile = tile_of_point(lo);
                // Area owners still hold authority over plots inside their own area, unless only
                // plot visitors are restricted.
                let is_area_owner = self.area_policy(tile).is_some_and(|a| a.owners.iter().any(|o| o == player));
                if !is_area_owner {
                    let pol = plot.visitor_policy.unwrap_or_else(RolePolicy::read_only);
                    check_policy(tile, player, &pol, action, &format!("plot \"{}\"", plot.name))?;
                }
            }
        }

        if let Some((role, pol)) = granted_by_plot {
            if role == Role::Owner {
                return Ok(());
            }
            let tile = tile_of_point(lo);
            return check_policy(tile, player, &pol, action, "plot");
        }

        // 2. Check every tile overlapped by `lo..hi`.
        let gx0 = (lo[0] / WIDTH).floor() as i32;
        let gx1 = (hi[0] / WIDTH).floor() as i32;
        let gz0 = (lo[2] / WIDTH).floor() as i32;
        let gz1 = (hi[2] / WIDTH).floor() as i32;

        for gx in gx0..=gx1 {
            for gz in gz0..=gz1 {
                let tile = [gx, gz];
                let Some(area) = self.area_policy(tile) else { continue };
                let role = area.role_of(player);
                if role == Role::Owner {
                    continue;
                }
                let pol = match role {
                    Role::Owner => RolePolicy::full(),
                    Role::Builder => area.builder_policy,
                    Role::Visitor => area.visitor_policy,
                };
                let label = format!("area ({}, {})", tile[0], tile[1]);
                check_policy(tile, player, &pol, action, &label)?;

                // Border & spawn guards protect shared seams and spawn points from non-owner terrain edits.
                if matches!(action, Action::Sculpt { .. } | Action::ModifyWater { .. }) {
                    let tx0 = gx as f32 * WIDTH;
                    let tz0 = gz as f32 * WIDTH;
                    let tx1 = tx0 + WIDTH;
                    let tz1 = tz0 + WIDTH;
                    if area.border_guard > 0.0 {
                        let g = area.border_guard;
                        let cl_x0 = lo[0].max(tx0);
                        let cl_x1 = hi[0].min(tx1);
                        let cl_z0 = lo[2].max(tz0);
                        let cl_z1 = hi[2].min(tz1);
                        if cl_x0 < tx0 + g || cl_x1 > tx1 - g || cl_z0 < tz0 + g || cl_z1 > tz1 - g {
                            return Err(Denial {
                                tile,
                                reason: format!("border zone of {label} is protected (within {:.1} m of edge)", area.border_guard),
                            });
                        }
                    }
                    if area.spawn_guard > 0.0 {
                        let sx = tx0 + START[0] as f32;
                        let sz = tz0 + START[1] as f32;
                        let dx = (sx - lo[0].max(sx.min(hi[0]))).abs();
                        let dz = (sz - lo[2].max(sz.min(hi[2]))).abs();
                        if dx * dx + dz * dz < area.spawn_guard * area.spawn_guard {
                            return Err(Denial {
                                tile,
                                reason: format!("spawn zone of {label} is protected"),
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        for a in &self.areas {
            a.validate()?;
        }
        for p in &self.plots {
            p.validate()?;
        }
        Ok(())
    }

    /// Serializes all `policy` and `plot` records for `WorldSave`.
    pub fn write_records(&self, out: &mut String) {
        for a in &self.areas {
            let owners = if a.owners.is_empty() { "-".into() } else { a.owners.join(",") };
            let builders = if a.builders.is_empty() { "-".into() } else { a.builders.join(",") };
            out.push_str(&format!(
                "policy {} {} owners {} builders {} builder {} visitor {} border {} spawn {}\n",
                a.tile[0],
                a.tile[1],
                owners,
                builders,
                a.builder_policy.encode(),
                a.visitor_policy.encode(),
                a.border_guard,
                a.spawn_guard
            ));
        }
        for p in &self.plots {
            let owners = if p.owners.is_empty() { "-".into() } else { p.owners.join(",") };
            let builders = if p.builders.is_empty() { "-".into() } else { p.builders.join(",") };
            let vis = p.visitor_policy.map_or_else(|| "none".into(), |v| v.encode());
            out.push_str(&format!(
                "plot {} {} lo {} {} {} hi {} {} {} owners {} builders {} builder {} visitor {}\n",
                p.id,
                p.name,
                p.lo[0],
                p.lo[1],
                p.lo[2],
                p.hi[0],
                p.hi[1],
                p.hi[2],
                owners,
                builders,
                p.builder_policy.encode(),
                vis
            ));
        }
    }

    pub fn parse_policy_line(line: &str, line_no: usize) -> Result<AreaPolicy> {
        let tok: Vec<&str> = line.split_whitespace().collect();
        if tok.len() != 15 {
            bail!("line {line_no}: expected 15 tokens in policy record, got {}", tok.len());
        }
        let expect = |i: usize, kw: &str| -> Result<()> {
            if tok[i] == kw { Ok(()) } else { bail!("line {line_no}: expected `{kw}`, got `{}`", tok[i]) }
        };
        let gx: i32 = tok[1].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad policy gx: {e}"))?;
        let gz: i32 = tok[2].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad policy gz: {e}"))?;
        expect(3, "owners")?;
        let owners = parse_list(tok[4]);
        expect(5, "builders")?;
        let builders = parse_list(tok[6]);
        expect(7, "builder")?;
        let builder_policy = RolePolicy::decode(tok[8])?;
        expect(9, "visitor")?;
        let visitor_policy = RolePolicy::decode(tok[10])?;
        expect(11, "border")?;
        let border_guard: f32 = tok[12].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad border_guard: {e}"))?;
        expect(13, "spawn")?;
        let spawn_guard: f32 = tok[14].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad spawn_guard: {e}"))?;
        let policy = AreaPolicy { tile: [gx, gz], owners, builders, builder_policy, visitor_policy, border_guard, spawn_guard };
        policy.validate()?;
        Ok(policy)
    }

    pub fn parse_plot_line(line: &str, line_no: usize) -> Result<Plot> {
        let tok: Vec<&str> = line.split_whitespace().collect();
        if tok.len() != 19 {
            bail!("line {line_no}: expected 19 tokens in plot record, got {}", tok.len());
        }
        let expect = |i: usize, kw: &str| -> Result<()> {
            if tok[i] == kw { Ok(()) } else { bail!("line {line_no}: expected `{kw}`, got `{}`", tok[i]) }
        };
        let pf = |i: usize| -> Result<f32> { tok[i].parse::<f32>().map_err(|e| wasmtime::format_err!("line {line_no}: bad float: {e}")) };
        let id: u32 = tok[1].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad plot id: {e}"))?;
        let name = tok[2].to_string();
        expect(3, "lo")?;
        let lo = [pf(4)?, pf(5)?, pf(6)?];
        expect(7, "hi")?;
        let hi = [pf(8)?, pf(9)?, pf(10)?];
        expect(11, "owners")?;
        let owners = parse_list(tok[12]);
        expect(13, "builders")?;
        let builders = parse_list(tok[14]);
        expect(15, "builder")?;
        let builder_policy = RolePolicy::decode(tok[16])?;
        expect(17, "visitor")?;
        let visitor_policy = if tok[18] == "none" { None } else { Some(RolePolicy::decode(tok[18])?) };
        let plot = Plot { id, name, lo, hi, owners, builders, builder_policy, visitor_policy };
        plot.validate()?;
        Ok(plot)
    }
}

fn tile_of_point(p: [f32; 3]) -> [i32; 2] {
    [(p[0] / WIDTH).floor() as i32, (p[2] / WIDTH).floor() as i32]
}

fn size_allowed(max: Option<Size>, player_size: f32) -> bool {
    match max {
        None => false,
        Some(limit) => player_size <= limit.height() * 1.15,
    }
}

fn check_policy(tile: [i32; 2], player: &str, pol: &RolePolicy, action: &Action<'_>, scope: &str) -> std::result::Result<(), Denial> {
    let deny = |reason: String| Err(Denial { tile, reason });
    match *action {
        Action::Visit { .. } => {
            if !pol.visit {
                return deny(format!("entry into {scope} is not allowed"));
            }
        }
        Action::Interact { .. } => {
            if !pol.interact {
                return deny(format!("interacting in {scope} is not allowed"));
            }
        }
        Action::Sculpt { player_size, is_water, .. } => {
            let cap = if is_water { pol.water_max } else { pol.sculpt_max };
            if !size_allowed(cap, player_size) {
                return match cap {
                    None => deny(format!("{} in {scope} is not allowed", if is_water { "editing water" } else { "sculpting" })),
                    Some(max) => deny(format!("sculpting in {scope} is limited to {} scale or smaller", max.name())),
                };
            }
        }
        Action::ModifyWater { player_size, .. } => {
            if !size_allowed(pol.water_max, player_size) {
                return match pol.water_max {
                    None => deny(format!("modifying water in {scope} is not allowed")),
                    Some(max) => deny(format!("modifying water in {scope} is limited to {} scale or smaller", max.name())),
                };
            }
        }
        Action::PlaceElement { player_size, .. } => {
            if !size_allowed(pol.place_max, player_size) {
                return match pol.place_max {
                    None => deny(format!("placing elements in {scope} is not allowed")),
                    Some(max) => deny(format!("placing elements in {scope} is limited to {} scale or smaller", max.name())),
                };
            }
        }
        Action::RemoveElement { owner, .. } | Action::CutOrCarry { owner, .. } => match pol.cut_carry {
            CutCarryPolicy::Any => {}
            CutCarryPolicy::OwnOnly => {
                if owner != Some(player) {
                    return deny(format!("in {scope} you can only modify objects you own"));
                }
            }
            CutCarryPolicy::None => {
                return deny(format!("chopping, carrying, or removing objects in {scope} is not allowed"));
            }
        },
        Action::SculptElement { player_size, owner, .. } => {
            if !size_allowed(pol.sculpt_max, player_size) {
                return match pol.sculpt_max {
                    None => deny(format!("sculpting objects in {scope} is not allowed")),
                    Some(max) => deny(format!("sculpting objects in {scope} is limited to {} scale or smaller", max.name())),
                };
            }
            if pol.cut_carry == CutCarryPolicy::OwnOnly && owner != Some(player) {
                return deny(format!("in {scope} you can only sculpt objects you own"));
            }
            if pol.cut_carry == CutCarryPolicy::None {
                return deny(format!("modifying objects in {scope} is not allowed"));
            }
        }
    }
    Ok(())
}

fn validate_principals(list: &[String], label: &str) -> Result<()> {
    for id in list {
        if id.is_empty() || id == "-" || id.contains(',') || id.chars().any(char::is_whitespace) {
            bail!("invalid player id `{id}` in {label}");
        }
    }
    Ok(())
}

fn parse_list(s: &str) -> Vec<String> {
    if s == "-" || s.is_empty() {
        Vec::new()
    } else {
        s.split(',').filter(|p| !p.is_empty()).map(String::from).collect()
    }
}
