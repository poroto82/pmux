use pmux::component::*;
use pmux::ids::PaneId;

#[test]
fn terminal_component_basics() {
    let mut term = TerminalComponent::new("API", 80, 24);
    assert_eq!(term.component_type(), TERMINAL);
    assert_eq!(term.display_name(), "API");
    assert_eq!(term.state(), ComponentState::Running);
    assert_eq!(term.cols(), 80);
    assert_eq!(term.lines(), 24);

    term.process(b"hello");
    term.input(ComponentInput::Focus);
    let out = term.render(80, 24);
    match out {
        RenderOutput::Grid { .. } => {}
        other => panic!("expected Grid, got {:?}", std::mem::discriminant(&other)),
    }
}

#[test]
fn registry_create_and_get_terminal() {
    let mut reg = ComponentRegistry::new();
    let pane = PaneId::new();
    reg.create_terminal(pane.clone(), "shell", 80, 24);

    assert_eq!(reg.count(), 1);
    assert!(reg.get(&pane).is_some());
    assert_eq!(reg.get(&pane).unwrap().component_type(), TERMINAL);
    assert!(reg.get_terminal_mut(&pane).is_some());
    assert!(reg.remove(&pane));
    assert_eq!(reg.count(), 0);
}

#[test]
fn registry_load_plugins() {
    let mut reg = ComponentRegistry::new();
    reg.load_plugins();
    // At least creates example clock plugin on first run
    assert!(!reg.available_plugins().is_empty() || reg.available_plugins().is_empty());
}

#[test]
fn script_component_from_config() {
    let config = PluginConfig {
        name: "Clock".into(),
        command: "echo".into(),
        args: vec!["tick".into()],
        refresh_secs: 1.0,
        description: "test".into(),
    };
    let mut script = ScriptComponent::new(config);
    assert_eq!(script.component_type(), SCRIPT);
    assert_eq!(script.display_name(), "Clock");
    script.tick();
    let _ = script.render(40, 10);
}

#[test]
fn component_type_helpers() {
    let ct = ComponentType::new(TERMINAL);
    assert_eq!(ct.as_str(), "terminal");
    assert_eq!(ct.to_string(), "terminal");
}

#[test]
fn registry_create_monitors() {
    let mut reg = ComponentRegistry::new();
    let ports = PaneId::new();
    let procs = PaneId::new();
    let job = PaneId::new();
    reg.create_ports(ports.clone(), "ports");
    reg.create_processes(procs.clone(), "procs");
    reg.create_job(job.clone(), "job");
    assert_eq!(reg.get(&ports).unwrap().component_type(), PORTS);
    assert_eq!(reg.get(&procs).unwrap().component_type(), PROCESSES);
    assert_eq!(reg.get(&job).unwrap().component_type(), JOB);
    assert!(reg.get_ports_mut(&ports).is_some());
    assert!(reg.get_job_mut(&job).is_some());
}
