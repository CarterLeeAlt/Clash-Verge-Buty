use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use windows::core::BSTR;
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::NetworkManagement::WindowsFirewall::{
    INetFwPolicy2, INetFwRule, INetFwRules, NetFwPolicy2, NetFwRule, NET_FW_ACTION_ALLOW,
    NET_FW_IP_PROTOCOL_ANY, NET_FW_PROFILE2_ALL, NET_FW_PROFILE2_DOMAIN,
    NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC, NET_FW_RULE_DIR_IN,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_APARTMENTTHREADED,
};

const RULE_PREFIX: &str = "Clash-Verge-Buty LAN";
const RULE_GROUP: &str = "Clash-Verge-Buty";
const CORE_BINARIES: [&str; 2] = ["mihomo.exe", "mihomo-alpha.exe"];

struct ComGuard;

impl ComGuard {
    fn initialize() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .context("failed to initialize COM for Windows Firewall")?;
        }
        Ok(Self)
    }
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

fn install_dir() -> Result<PathBuf> {
    let helper = std::env::current_exe().context("failed to resolve firewall-helper.exe path")?;
    let resources = helper
        .parent()
        .context("firewall-helper.exe has no parent directory")?;
    if !resources
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("resources"))
    {
        bail!("firewall-helper.exe must run from the application resources directory");
    }
    resources
        .parent()
        .map(PathBuf::from)
        .context("application resources directory has no parent")
}

fn rule_name(core: &str) -> String {
    format!("{RULE_PREFIX} - {core}")
}

fn firewall_rules() -> Result<INetFwRules> {
    unsafe {
        let policy: INetFwPolicy2 =
            CoCreateInstance(&NetFwPolicy2, None, CLSCTX_INPROC_SERVER)
                .context("failed to open Windows Firewall policy")?;
        policy
            .Rules()
            .context("failed to access Windows Firewall rules")
    }
}

fn remove_rule_if_present(rules: &INetFwRules, name: &str) -> Result<()> {
    let name = BSTR::from(name);
    unsafe {
        while rules.Item(&name).is_ok() {
            rules
                .Remove(&name)
                .context("failed to remove a managed Windows Firewall rule")?;
        }
    }
    Ok(())
}

fn add_rule(rules: &INetFwRules, name: &str, program: &Path) -> Result<()> {
    let program = program.to_string_lossy().into_owned();
    let expected_name = BSTR::from(name);
    let expected_program = BSTR::from(program.as_str());

    unsafe {
        let rule: INetFwRule = CoCreateInstance(&NetFwRule, None, CLSCTX_INPROC_SERVER)
            .context("failed to create a Windows Firewall rule")?;
        rule.SetName(&expected_name)?;
        rule.SetDescription(&BSTR::from(
            "Allows local-subnet devices to use the Mihomo proxy listener.",
        ))?;
        rule.SetGrouping(&BSTR::from(RULE_GROUP))?;
        rule.SetApplicationName(&expected_program)?;
        rule.SetProtocol(NET_FW_IP_PROTOCOL_ANY.0)?;
        rule.SetDirection(NET_FW_RULE_DIR_IN)?;
        rule.SetEnabled(VARIANT_BOOL::from(true))?;
        rule.SetProfiles(NET_FW_PROFILE2_ALL.0)?;
        rule.SetRemoteAddresses(&BSTR::from("LocalSubnet"))?;
        rule.SetEdgeTraversal(VARIANT_BOOL::from(false))?;
        rule.SetAction(NET_FW_ACTION_ALLOW)?;
        rules
            .Add(&rule)
            .with_context(|| format!("failed to add Windows Firewall rule '{name}'"))?;

        let stored = rules
            .Item(&expected_name)
            .with_context(|| format!("Windows Firewall rule '{name}' was not found after creation"))?;
        let stored_program = stored.ApplicationName()?.to_string();
        let required_profiles =
            NET_FW_PROFILE2_DOMAIN.0 | NET_FW_PROFILE2_PRIVATE.0 | NET_FW_PROFILE2_PUBLIC.0;
        let valid = stored_program.eq_ignore_ascii_case(&program)
            && stored.Enabled()?.as_bool()
            && stored.Action()? == NET_FW_ACTION_ALLOW
            && stored.Direction()? == NET_FW_RULE_DIR_IN
            && stored.Protocol()? == NET_FW_IP_PROTOCOL_ANY.0
            && stored.Profiles()? & required_profiles == required_profiles
            && stored
                .RemoteAddresses()?
                .to_string()
                .eq_ignore_ascii_case("LocalSubnet")
            && !stored.EdgeTraversal()?.as_bool();
        if !valid {
            bail!("Windows Firewall rule '{name}' did not match the requested configuration");
        }
    }
    Ok(())
}

fn remove_managed_rules(rules: &INetFwRules) -> Result<()> {
    for core in CORE_BINARIES {
        remove_rule_if_present(rules, &rule_name(core))?;
    }
    Ok(())
}

fn enable_rules(rules: &INetFwRules) -> Result<()> {
    let install_dir = install_dir()?;
    let cores = CORE_BINARIES
        .iter()
        .map(|core| (rule_name(core), install_dir.join(core)))
        .collect::<Vec<_>>();

    for (_, core) in &cores {
        if !core.is_file() {
            bail!("Mihomo core was not found: {}", core.display());
        }
    }

    remove_managed_rules(rules)?;
    for (name, core) in &cores {
        if let Err(err) = add_rule(rules, name, core) {
            let _ = remove_managed_rules(rules);
            return Err(err);
        }
    }
    Ok(())
}

fn run() -> Result<()> {
    let operation = std::env::args()
        .nth(1)
        .context("missing operation; expected --enable-lan or --disable-lan")?;
    let _com = ComGuard::initialize()?;
    let rules = firewall_rules()?;

    match operation.as_str() {
        "--enable-lan" => enable_rules(&rules),
        "--disable-lan" => remove_managed_rules(&rules),
        _ => bail!("unsupported firewall operation"),
    }
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::{rule_name, CORE_BINARIES};

    #[test]
    fn uses_stable_distinct_rule_names() {
        assert_eq!(
            rule_name(CORE_BINARIES[0]),
            "Clash-Verge-Buty LAN - mihomo.exe"
        );
        assert_eq!(
            rule_name(CORE_BINARIES[1]),
            "Clash-Verge-Buty LAN - mihomo-alpha.exe"
        );
    }
}
