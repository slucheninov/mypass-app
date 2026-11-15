use mypass_platform::windows_registry::*;

#[test]
fn windows_registry_typed_reads_views_and_owned_deletion() {
    let r = MemoryRegistry::default();
    let k = Key::new(Hive::CurrentUser, View::Registry64, "Software\\Example").unwrap();
    let other = Key::new(Hive::CurrentUser, View::Registry32, k.subkey()).unwrap();
    assert_eq!(r.read(&k, "", ValueType::String).unwrap(), Read::Missing);
    r.create(&k).unwrap();
    let owner = Value::String("owner".into());
    r.set(&k, "", owner.clone()).unwrap();
    assert_eq!(r.read(&k, "", ValueType::Binary).unwrap(), Read::WrongType);
    assert_eq!(
        r.read(&other, "", ValueType::String).unwrap(),
        Read::Missing
    );
    assert!(!r.delete_owned_key(&k, "", &Value::Binary(vec![])).unwrap());
    assert!(r.delete_owned_key(&k, "", &owner).unwrap());
    let machine = Key::new(Hive::LocalMachine, View::Registry64, "Software\\Example").unwrap();
    assert!(r.create(&machine).is_err());
}

#[test]
fn windows_registry_isolation_cannot_escape_test_root() {
    let r = IsolatedRegistry::new(MemoryRegistry::default());
    assert!(r.root().starts_with("Software\\MyPassTests\\"));
    let k = Key::new(Hive::LocalMachine, View::Registry64, "Google\\Chrome").unwrap();
    r.create(&k).unwrap(); // even HKLM writes are redirected to the private HKCU root
    r.set(&k, "pv", Value::Binary(vec![1, 2])).unwrap();
    assert_eq!(
        r.read(&k, "PV", ValueType::Binary).unwrap(),
        Read::Value(Value::Binary(vec![1, 2]))
    );
    for invalid in [
        "",
        "\\absolute",
        "..\\escape",
        "Software/escape",
        "x\\\\y",
        "x\0y",
    ] {
        assert!(Key::new(Hive::CurrentUser, View::Registry64, invalid).is_err());
    }
}

#[test]
fn windows_targets_always_include_chrome_and_edge_and_paths_are_exact() {
    use mypass_platform::{windows_host_key, WINDOWS_REGISTRY_TARGETS};
    for (id, vendor) in [("chrome", "Google\\Chrome"), ("edge", "Microsoft\\Edge")] {
        let target = WINDOWS_REGISTRY_TARGETS
            .iter()
            .find(|t| t.id == id)
            .unwrap();
        assert!(target.always);
        assert_eq!(
            windows_host_key(target, View::Registry32).unwrap(),
            Key::new(
                Hive::CurrentUser,
                View::Registry32,
                &format!("Software\\{vendor}\\NativeMessagingHosts\\com.mypass.chromehost")
            )
            .unwrap()
        );
    }
    assert_eq!(WINDOWS_REGISTRY_TARGETS.len(), 3);
    assert!(WINDOWS_REGISTRY_TARGETS
        .iter()
        .all(|t| !t.subkey.to_lowercase().contains("wow6432node")));
}

#[test]
fn registry_owned_delete_preserves_named_values_and_children() {
    let r = MemoryRegistry::default();
    let key = Key::new(Hive::CurrentUser, View::Registry64, "Software\\host").unwrap();
    r.create(&key).unwrap();
    let owner = Value::String("ours".into());
    r.set(&key, "", owner.clone()).unwrap();
    r.set(&key, "foreign", Value::Binary(vec![42])).unwrap();
    assert!(r.delete_owned_key(&key, "", &owner).unwrap());
    assert_eq!(r.read(&key, "", ValueType::String).unwrap(), Read::Missing);
    assert_eq!(
        r.read(&key, "foreign", ValueType::Binary).unwrap(),
        Read::Value(Value::Binary(vec![42]))
    );
    assert!(r.key_exists(&key).unwrap());
}
