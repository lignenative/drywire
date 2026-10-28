//! The commands about this device's identity, its spaces and invitations:
//! everything but talking, which is `join`.

use drywire_control::{Client, SpaceView};
use drywire_proto::invite::Invitation;

use crate::args::Account;
use crate::home::{Home, KnownSpace, printable};

/// A client for the space's instance, with this device's identity.
fn client(home: &Home, instance: &str) -> Result<Client, String> {
    Client::new(instance, home.identity()?).map_err(|e| e.to_string())
}

/// `identity`: prints the device's public key, which an operator lists in
/// DRYWIRE_SPACE_CREATORS to let this device create spaces.
pub fn identity(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let key = home.identity()?.verifying_key();
    println!("{}", drywire_control::to_hex(key.as_bytes()));
    Ok(())
}

/// `create-space`: a new space, with one room and a fresh space key.
pub fn create_space(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let instance = args
        .instance
        .ok_or("create-space needs --instance https://HOST")?;
    let name = args.name.ok_or("create-space needs --name NAME")?;
    let client = client(&home, &instance)?;
    let view = client
        .create_space(
            &name,
            args.room.as_deref().unwrap_or("general"),
            &args.display_name,
        )
        .map_err(|e| e.to_string())?;
    // 256 random bits that only the members will ever hold.
    let mut space_key = [0u8; 32];
    getrandom::fill(&mut space_key).map_err(|e| format!("random bytes: {e}"))?;
    home.remember(KnownSpace::new(
        client.instance(),
        view.space.id,
        &view.space.name,
        &space_key,
    ))?;
    println!(
        "Created {} on {} with the room {}. Invite people with: drywire invite --space {:?}",
        printable(&view.space.name),
        client.instance(),
        printable(&view.rooms[0].name),
        printable(&view.space.name)
    );
    Ok(())
}

/// `invite`: prints an invitation link to a space.
pub fn invite(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let space = home.find(args.space.as_deref())?;
    let client = client(&home, &space.instance)?;
    let expires = args.days.map(|d| (d * 86_400.0) as u64);
    let (invitation, info) = client
        .invite(&space.space_id, &space.space_key()?, expires, args.uses)
        .map_err(|e| e.to_string())?;
    eprintln!(
        "An invitation to {} for {} use(s), until {}. It holds the space key: share it \
         privately.",
        space.name, info.max_uses, info.expires_at
    );
    println!("{}", invitation.to_link());
    Ok(())
}

/// Redeems a link and remembers the space; shared by `accept` and
/// `join --link`.
pub fn accept_link(home: &Home, link: &str, display_name: &str) -> Result<KnownSpace, String> {
    let invitation = Invitation::parse(link).map_err(|e| e.to_string())?;
    let client = client(home, &invitation.instance)?;
    let view: SpaceView = client
        .redeem(&invitation.code, display_name)
        .map_err(|e| e.to_string())?;
    let space = KnownSpace::new(
        client.instance(),
        view.space.id,
        &view.space.name,
        &invitation.space_key,
    );
    home.remember(space.clone())?;
    Ok(space)
}

/// `accept LINK`: becomes a member of the space the link is for.
pub fn accept(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let link = args.value.unwrap_or_default();
    let space = accept_link(&home, &link, &args.display_name)?;
    println!(
        "You are a member of {} on {}. Talk with: drywire join --space {:?}",
        space.name, space.instance, space.name
    );
    Ok(())
}

/// `spaces`: the spaces this device knows, with their rooms.
pub fn spaces(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    for space in home.spaces()? {
        let rooms = Client::new(&space.instance, home.identity()?)
            .and_then(|c| c.space(&space.space_id))
            .map(|v| {
                let rooms: Vec<String> = v.rooms.iter().map(|r| printable(&r.name)).collect();
                format!("{}, rooms: {}", printable(&v.role), rooms.join(", "))
            })
            .unwrap_or_else(|e| e.to_string());
        println!(
            "{}  {} on {} ({rooms})",
            space.space_id, space.name, space.instance
        );
    }
    Ok(())
}

/// The instances this device knows, or the one `--instance` names.
fn instances(home: &Home, args: &Account) -> Result<Vec<String>, String> {
    if let Some(i) = &args.instance {
        return Ok(vec![i.clone()]);
    }
    let mut all: Vec<String> = home.spaces()?.into_iter().map(|s| s.instance).collect();
    all.sort();
    all.dedup();
    if all.is_empty() {
        return Err("this device knows no instance: pass --instance https://HOST".into());
    }
    Ok(all)
}

/// `export`: everything each instance stores about this identity, as JSON.
pub fn export(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let mut out = serde_json::Map::new();
    for instance in instances(&home, &args)? {
        let data = client(&home, &instance)?
            .export()
            .map_err(|e| format!("{instance}: {e}"))?;
        out.insert(instance, data);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&out).expect("serializable")
    );
    Ok(())
}

/// `rename NAME`: the display name, on every instance.
pub fn rename(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let name = args.value.clone().unwrap_or_default();
    for instance in instances(&home, &args)? {
        let id = client(&home, &instance)?
            .rename(&name)
            .map_err(|e| format!("{instance}: {e}"))?;
        println!("{}: you are now {}", instance, printable(&id.name));
    }
    Ok(())
}

/// `leave-space`: leaves a space and forgets its key.
pub fn leave_space(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let space = home.find(args.space.as_deref())?;
    client(&home, &space.instance)?
        .leave_space(&space.space_id)
        .map_err(|e| e.to_string())?;
    home.forget(&space.instance, Some(&space.space_id))?;
    println!("You left {}.", space.name);
    Ok(())
}

/// `delete-identity`: deletes this identity from every instance it knows
/// (or the one named), then forgets it on this device.
pub fn delete_identity(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let all = instances(&home, &args)?;
    for instance in &all {
        client(&home, instance)?
            .delete_identity()
            .map_err(|e| format!("{instance}: {e}"))?;
        home.forget(instance, None)?;
        println!("{instance}: identity deleted");
    }
    if args.instance.is_none() {
        home.forget_identity()?;
        println!("This device's key is forgotten; a new one is made on next use.");
    }
    Ok(())
}
