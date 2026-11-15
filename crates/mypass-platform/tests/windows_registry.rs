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
