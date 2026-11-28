#[allow(
    clippy::disallowed_methods,
    reason = "keyboxd's database is not the store"
)]
pub async fn openpgp_keyblobs(path: &std::path::Path) -> Result<Vec<Vec<u8>>, String> {
    let path = path.to_str().ok_or("the keyring's path is not UTF-8")?;
    let database = turso::Builder::new_local(path)
        .read_only(true)
        .build()
        .await
        .map_err(|error| error.to_string())?;
    let connection = database.connect().map_err(|error| error.to_string())?;
    let mut rows = connection
        .query(
            "SELECT keyblob FROM pubkey WHERE ephemeral = 0 AND type = 1",
            (),
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|error| error.to_string())? {
        if let Ok(blob) = row.get::<Vec<u8>>(0) {
            out.push(blob);
        }
    }
    Ok(out)
}
