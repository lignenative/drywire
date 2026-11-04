//! The commands about this device's identity, its spaces and invitations:
//! everything but talking, which is `join`.

use drywire_control::{Client, Hex, SpaceView, VaultContents, VaultKind, VaultSpace};
use drywire_proto::invite::Invitation;
use drywire_proto::vault::VaultCode;

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
        .create_space_with_code(
            &name,
            args.room.as_deref().unwrap_or("general"),
            &args.display_name,
            args.code.as_deref(),
        )
        .map_err(|e| e.to_string())?;
    // An account signed in by e-mail gets a space with an MLS group,
    // made at its first call (ADRs 0052 and 0055); a key-only identity,
    // a space key in its links.
    let signed_in = client
        .me()
        .map_err(|e| e.to_string())?
        .and_then(|me| me.username)
        .is_some_and(|name| !name.is_empty());
    if signed_in {
        home.remember(KnownSpace::with_group(
            client.instance(),
            view.space.id,
            &view.space.name,
        ))?;
    } else {
        // 256 random bits that only the members will ever hold.
        let mut space_key = [0u8; 32];
        getrandom::fill(&mut space_key).map_err(|e| format!("random bytes: {e}"))?;
        home.remember(KnownSpace::new(
            client.instance(),
            view.space.id,
            &view.space.name,
            &space_key,
        ))?;
    }
    seal_recovery_again(&home);
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
    if let Some(user) = &args.user {
        let found = client
            .invite_user(&space.space_id, user)
            .map_err(|e| e.to_string())?;
        println!(
            "Invited {} ({}) into {}: the invitation waits in their app for 30 days.",
            printable(&found.username),
            printable(&found.name),
            space.name
        );
        return Ok(());
    }
    if space.uses_group() {
        return Err("this space invites by user name: drywire invite --user NAME".into());
    }
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

/// The instance a command acts on: `--instance`, else the first one this
/// device knows.
fn instance_of(home: &Home, args: &Account) -> Result<String, String> {
    if let Some(instance) = &args.instance {
        return Ok(instance.clone());
    }
    home.spaces()?
        .first()
        .map(|s| s.instance.clone())
        .ok_or_else(|| "say which instance with --instance https://HOST".into())
}

/// `signin EMAIL`: a code is mailed to the address; with it, this device
/// signs in to that account, or makes it (ADR 0055). Its spaces are then
/// known here.
pub fn sign_in(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let instance = args
        .instance
        .clone()
        .ok_or("signin needs --instance https://HOST")?;
    let email = args.value.clone().ok_or("signin needs an e-mail address")?;
    let client = client(&home, &instance)?;
    let code = match &args.code {
        Some(code) => code.clone(),
        None => {
            client
                .request_email_code(&email, "en")
                .map_err(|e| e.to_string())?;
            eprint!("A 6-digit code was mailed to {email}. Code: ");
            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            line
        }
    };
    let signed = client
        .verify_email_code(&email, code.trim())
        .map_err(|e| e.to_string())?;
    let spaces = remember_memberships(&home, &client)?;
    match signed.username {
        Some(name) => println!(
            "Signed in as {}; {spaces} space(s) known here.",
            printable(&name)
        ),
        None => println!(
            "Account made. Choose your user name: drywire username NAME --instance {instance}"
        ),
    }
    Ok(())
}

/// Remembers the spaces the account is a member of that this device did
/// not know: their keys come from their groups.
fn remember_memberships(home: &Home, client: &Client) -> Result<usize, String> {
    let known = home.spaces()?;
    let mut count = 0;
    for m in client.spaces().map_err(|e| e.to_string())? {
        count += 1;
        if !known
            .iter()
            .any(|k| k.instance == client.instance() && k.space_id == m.space.id)
        {
            home.remember(KnownSpace::with_group(
                client.instance(),
                m.space.id,
                &m.space.name,
            ))?;
        }
    }
    Ok(count)
}

/// `username NAME`: chooses the account's user name.
pub fn username(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let client = client(&home, &instance_of(&home, &args)?)?;
    let name = args.value.clone().ok_or("username needs a NAME")?;
    let display = if args.display_name == crate::args::default_display_name() {
        ""
    } else {
        &args.display_name
    };
    match client.set_username(&name, display) {
        Ok(me) => {
            println!(
                "You are {} now.",
                printable(me.username.as_deref().unwrap_or(&name))
            );
            Ok(())
        }
        Err(drywire_control::Error::UsernameTaken { suggestions }) => Err(format!(
            "this user name is taken; free ones: {}",
            suggestions.join(", ")
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// `invitations`: the spaces this account is invited to.
pub fn invitations(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let client = client(&home, &instance_of(&home, &args)?)?;
    let list = client.invitations().map_err(|e| e.to_string())?;
    if list.is_empty() {
        println!("No invitation waits.");
    }
    for i in list {
        println!(
            "{}  {}  from {}",
            i.space_id,
            printable(&i.space_name),
            printable(&i.invited_by)
        );
    }
    Ok(())
}

/// `accept-invitation SPACE`: joins a space one was invited to, by its id
/// or its name as `invitations` lists them.
pub fn accept_invitation(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let client = client(&home, &instance_of(&home, &args)?)?;
    let wanted = args
        .value
        .clone()
        .ok_or("accept-invitation needs a SPACE")?;
    let list = client.invitations().map_err(|e| e.to_string())?;
    let found = list
        .iter()
        .find(|i| i.space_id.to_string() == wanted || i.space_name.eq_ignore_ascii_case(&wanted))
        .ok_or("no invitation to that space")?;
    let view = client
        .accept_invitation(&found.space_id)
        .map_err(|e| e.to_string())?;
    home.remember(KnownSpace::with_group(
        client.instance(),
        view.space.id,
        &view.space.name,
    ))?;
    println!(
        "You are in {}. Its keys come at your first call, once a member's app lets this device in.",
        printable(&view.space.name)
    );
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
    seal_recovery_again(home);
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
    seal_recovery_again(&home);
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

/// What this device would put in a vault: its account and spaces. The
/// name is the one `client`'s instance knows.
fn vault_contents(home: &Home, client: &Client) -> Result<VaultContents, String> {
    let identity = home.identity()?;
    let spaces = home.spaces()?;
    let name = client
        .me()
        .ok()
        .flatten()
        .map(|id| id.name)
        .unwrap_or_else(crate::args::default_display_name);
    // A space with an MLS group has no key to carry: a new device is let
    // into its group instead (ADR 0055).
    let spaces = spaces
        .iter()
        .filter(|s| !s.uses_group())
        .map(|s| {
            Ok(VaultSpace {
                instance: s.instance.clone(),
                id: s.space_id,
                name: s.name.clone(),
                key: Hex(s.space_key()?),
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(VaultContents {
        account: Hex(identity.to_bytes()),
        name,
        spaces,
    })
}

/// The instances the recovery vault lives on: those of the spaces.
fn space_instances(home: &Home) -> Result<Vec<String>, String> {
    let mut all: Vec<String> = home.spaces()?.into_iter().map(|s| s.instance).collect();
    all.sort();
    all.dedup();
    Ok(all)
}

/// Seals the recovery vault with this device's spaces again, on every
/// instance, if this device knows the recovery code. A failure is told,
/// never fatal: the change it follows is done.
fn seal_recovery_again(home: &Home) {
    let result = (|| -> Result<(), String> {
        let Some(secret) = home.recovery_secret()? else {
            return Ok(());
        };
        let code = VaultCode::from_secret(secret);
        put_recovery_everywhere(home, &code)
    })();
    if let Err(e) = result {
        eprintln!(
            "warning: the recovery vault was not updated ({e}); run `drywire recovery-code` again"
        );
    }
}

/// Seals the recovery vault with `code` on every instance of the spaces,
/// one login each.
fn put_recovery_everywhere(home: &Home, code: &VaultCode) -> Result<(), String> {
    let clients = space_instances(home)?
        .iter()
        .map(|i| client(home, i))
        .collect::<Result<Vec<_>, _>>()?;
    let Some(first) = clients.first() else {
        return Ok(());
    };
    let contents = vault_contents(home, first)?;
    for c in &clients {
        c.put_recovery_vault(code, &contents)
            .map_err(|e| format!("{}: {e}", c.instance()))?;
    }
    Ok(())
}

/// `recovery-code`: makes a new recovery code, seals this account in a
/// vault on every instance of its spaces, and prints the code once. The
/// previous code opens nothing any more.
pub fn recovery_code(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let instances = space_instances(&home)?;
    if instances.is_empty() {
        return Err("this account has no space yet: join or create one first".into());
    }
    let code = drywire_control::new_code();
    put_recovery_everywhere(&home, &code)?;
    home.set_recovery_secret(Some(&code.secret()))?;
    eprintln!(
        "Your recovery code. Whoever has it has your account: keep it like a password, \
         offline. A new code replaces this one."
    );
    println!("{}", code.to_text());
    eprintln!(
        "On a new device: drywire restore CODE --instance {}",
        instances[0]
    );
    Ok(())
}

/// `device-code`: lets one new device become this account, for ten minutes.
pub fn device_code(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let instance = match args.instance {
        Some(i) => i,
        None => space_instances(&home)?
            .into_iter()
            .next()
            .ok_or("this account has no space yet: join or create one first")?,
    };
    let code = drywire_control::new_code();
    let client = client(&home, &instance)?;
    let contents = vault_contents(&home, &client)?;
    let waiting = client
        .put_device_vault(&code, &contents)
        .map_err(|e| e.to_string())?;
    println!("{}", code.to_text());
    eprintln!(
        "On the new device, within {} minutes: drywire restore CODE --instance {}",
        waiting.expires_in / 60,
        client.instance()
    );
    Ok(())
}

/// `restore CODE`: becomes the account a recovery or device code opens.
pub fn restore(args: Account) -> Result<(), String> {
    let home = Home::open(args.home.as_deref())?;
    let instance = args
        .instance
        .clone()
        .ok_or("restore needs --instance https://HOST, where the code was made")?;
    let text = args.value.clone().unwrap_or_default();
    let (kind, contents) =
        drywire_control::open_vault(&instance, &text).map_err(|e| e.to_string())?;
    let account = contents.signing_key();
    let current = home.spaces()?;
    let same = home.identity()?.verifying_key() == account.verifying_key();
    if !same && !current.is_empty() && !args.force {
        return Err(format!(
            "this device already has an account, with {} space(s): restore with --force to              replace it (its key is then forgotten here)",
            current.len()
        ));
    }
    let mut spaces = contents
        .spaces
        .iter()
        .map(|s| KnownSpace::new(&s.instance, s.id, &s.name, &s.key.0))
        .collect::<Vec<_>>();
    if same {
        // The same account: keep the spaces only this device knew.
        for s in current {
            if !spaces
                .iter()
                .any(|v| v.instance == s.instance && v.space_id == s.space_id)
            {
                spaces.push(s);
            }
        }
    }
    home.become_account(&account.to_bytes(), spaces)?;
    if kind == VaultKind::Recovery {
        // This device knows the code now: it keeps the vault up to date.
        let code = VaultCode::parse(&text).map_err(|e| e.to_string())?;
        home.set_recovery_secret(Some(&code.secret()))?;
    }
    println!(
        "You are {} on this device, with {} space(s). Talk with: drywire join",
        printable(&contents.name),
        contents.spaces.len()
    );
    Ok(())
}
