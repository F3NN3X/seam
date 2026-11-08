# Packaging and releases

Seam ships two ways from one build. The same `seam.exe` and `seam-settings.exe` go into both, and `is_packaged()` decides at runtime what differs.

| | Microsoft Store | GitHub |
|---|---|---|
| Format | MSIX from `pack-msix.ps1` | zip, raw exes and a signed `latest.json` from `release.yml` |
| Signing | Microsoft signs it during certification | unsigned, with a notice in the README |
| Updates | the Store | Seam's own updater, checked against a minisign key |

Everything here runs on Windows 11 with Rust and the Windows SDK. Nothing in `packaging/` needs Node or the MSIX Packaging Tool.

## Build the MSIX

```powershell
cargo build --release --workspace
pwsh packaging\pack-msix.ps1
```

This writes `dist\<IdentityName>_<version>_x64-store.msix`. It is unsigned, and it is the file for Partner Center. The version comes from the workspace `Cargo.toml` (`0.2.0` becomes `0.2.0.0`), and a pre-release tag is refused because the Store can't take one.

The `-store` and `-test` endings are there so the wrong file can't be uploaded.

## Try it on this PC

```powershell
pwsh packaging\pack-msix.ps1 -Sign
pwsh packaging\pack-msix.ps1 -Sign -Install
```

`-Sign` writes `dist\...-test.msix`, signed with a self-signed certificate whose subject is the manifest `Publisher`. The certificate has to be trusted in `LocalMachine\TrustedPeople`, which needs an elevated prompt once. `-Install` implies `-Sign`. It removes the old package by PackageFullName, then adds the new one. `Get-AppxPackage *Seam* | Remove-AppxPackage` skips a staged package and leaves `0x80073cfb` on the next install.

Then run the sandbox checks in `TESTING.md`, under "Microsoft Store build".

Remove the test package when you're done:

```powershell
Get-AppxPackage -Name <IdentityName> | Remove-AppxPackage
```

## Partner Center identity

Fill in `packaging\identity.json` from Product management, Product identity, once the product exists. The manifest must match these values exactly. The file isn't secret, so commit it.

| Field | From |
|---|---|
| `IdentityName` | Identity/Name |
| `Publisher` | Identity/Publisher (`CN=<GUID>`) |
| `PublisherDisplayName` | PublisherDisplayName |
| `StoreId` | the product's Store ID, for `-Upload` and `store.yml` |

Create the product as **MSIX/PWA**. It can't be changed later, and an EXE/MSI product gets no identity and no free signing.

`MaxVersionTested` in `AppxManifest.xml` is `10.0.26300.0`, the build this was tested on. Raise it when you test on a newer one.

## Upload a draft

The first submission is by hand in Partner Center. Choose "Don't publish until I select Publish now" so going live stays a click.

From the second release on, with the Microsoft Store Developer CLI:

```powershell
winget install "Microsoft Store Developer CLI"
msstore reconfigure --tenantId <tenant> --sellerId <seller> --clientId <client> --clientSecret <secret>
pwsh packaging\pack-msix.ps1 -Upload
```

`-Upload` runs `msstore publish <store.msix> --appId <StoreId> --noCommit`. That replaces the pending draft and leaves it as a draft. Look at the draft in Partner Center without editing it, because an API draft that was edited in the web UI can no longer be changed through the API. Then commit it:

```powershell
msstore submission publish <StoreId>
```

### Or from GitHub Actions

`.github/workflows/store.yml` does the same behind approval. Set it up once:

1. In Partner Center, under Account settings, Tenants, associate a Microsoft Entra tenant.
2. Under Users, Microsoft Entra applications, add an app with the **Manager** role and create a key. Note the Tenant ID, Client ID, the key (shown once, and it expires) and the Seller ID.
3. In GitHub, create the environment `store` with yourself as required reviewer, then:

```powershell
gh api -X PUT repos/F3NN3X/seam/environments/store
gh secret set PARTNER_CENTER_TENANT_ID --env store
gh secret set PARTNER_CENTER_SELLER_ID --env store
gh secret set PARTNER_CENTER_CLIENT_ID --env store
gh secret set PARTNER_CENTER_CLIENT_SECRET --env store
gh variable set STORE_APP_ID --env store --body <StoreId>
```

Add yourself as required reviewer under Settings, Environments, store. Then run the "Store" workflow from the Actions tab with the version. It packs, waits for your approval, uploads a draft, waits again, and commits to certification.

## The update key

The GitHub build installs an update only if Seam's minisign key (ID `DD3DF0F7CB3ACA88`) signed its `latest.json`. Its public half is `SEAM_PUBLIC_KEY` in `settings/src/update.rs`, and `release.yml` fails rather than draft a release nobody can update to. The key already exists; these are the steps for a new one. Install minisign and create the key once, on your machine, outside the repository:

```powershell
winget install jedisct1.minisign
# open a new terminal so minisign is on PATH
minisign -G -p seam.pub -s seam.key
```

It asks for a password. That password is `MINISIGN_PASSWORD` below.

Then:

1. Put the second line of `seam.pub` in `SEAM_PUBLIC_KEY` in `settings/src/update.rs`, and commit that.
2. Add both secrets to the repository:

```powershell
gh api -X PUT repos/F3NN3X/seam/environments/release
gh secret set MINISIGN_SECRET_KEY --env release < seam.key
gh secret set MINISIGN_PASSWORD --env release
```

The two secrets live in the `release` environment, not in the repository. Add yourself as required reviewer under Settings, Environments, release. Then a workflow edited on another branch can't read the key, and each tagged release waits for your approval before it signs.

3. Keep `seam.key` out of the repository, and delete it from this folder once it's in a password manager.

If the secret key might have been exposed, treat the key as lost: make a new pair, change `SEAM_PUBLIC_KEY`, and ship that in the next release. Builds that carry the old key stop trusting updates from the new one, so they need a manual download.

## Release on GitHub

1. Set `version` to the same number in `Cargo.toml` and `settings/Cargo.toml`. CI fails if the two differ.
2. In `CHANGELOG.md`, move **Unreleased** under a `## [X.Y.Z] - YYYY-MM-DD` heading. That section becomes the release notes, and its `- ` bullets become the notes the updater shows.
3. Merge that through a pull request, then tag the merge commit on `main` and push the tag:

```powershell
git tag v0.4.0
git push origin v0.4.0
```

4. `release.yml` checks the versions, the CHANGELOG section and that CI passed for that commit (or the pull request that merged it). Then it builds, writes and signs `latest.json`, and creates a **draft** release. Look at the draft, then publish it yourself.

The draft holds `seam-X.Y.Z-windows-x64.zip`, `seam.exe`, `seam-settings.exe`, `latest.json` and `latest.json.minisig`. A tag with a hyphen (`v0.3.0-beta.1`) makes a pre-release.
