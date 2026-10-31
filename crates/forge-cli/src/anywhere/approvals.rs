//! `forge anywhere approvals` and `safety-code`: list pending device approvals, and print a
//! pairing safety code without approving.

use super::*;

pub(super) async fn print_safety_code(encoded_challenge: &str) -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let access_token = ensure_access_token(&store, &mut state).await?;
    let config = forge_config::load()?;
    let service_url = config.anywhere.service_url();
    let challenge = parse_pairing_challenge(encoded_challenge, service_url, now_ms())?;
    let http = client()?;
    let details: PairingDetails = send_json(
        http.get(format!(
            "{service_url}/v1/pairings/{}",
            challenge.pairing_id
        ))
        .bearer_auth(&access_token),
    )
    .await
    .context("load device approval request")?;
    validate_pairing_details(&details, &challenge)?;
    let account_id = decode_hex_array::<16>(
        state.account_id.as_deref().context("missing account id")?,
        "account id",
    )?;
    let code = pairing_safety_code(&challenge, &details.signing_public_key, &account_id)?;
    println!("Safety code: {code}");
    Ok(())
}

pub(super) async fn list_approvals() -> Result<()> {
    let store = StateStore::platform()?;
    let mut state = store.load()?;
    let token = ensure_access_token(&store, &mut state).await?;
    let service_url = forge_config::load()?.anywhere.service_url().to_owned();
    let list: PairingList = send_json(
        client()?
            .get(format!("{service_url}/v1/pairings"))
            .bearer_auth(token),
    )
    .await
    .context("list pending device approvals")?;

    if list.pairings.is_empty() {
        println!("No enrollment requests are waiting for approval.");
        return Ok(());
    }
    let now = now_ms();
    println!("Enrollment requests waiting for approval:");
    for pairing in &list.pairings {
        let remaining = pairing.expires_at_ms.saturating_sub(now) / 1_000;
        println!(
            "  {}  expires in {remaining}s",
            safe_display_text(&pairing.device_name)
        );
    }
    println!();
    println!("Approve with: forge anywhere approve <CHALLENGE>");
    println!(
        "The challenge is shown on the device that is waiting — it is deliberately not listed"
    );
    println!("here, so a stolen account token cannot approve a device on its own.");
    Ok(())
}
