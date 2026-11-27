use crate::{
    ExecuteRequest, ExecuteResult, Kernel, KernelControl, KernelError, KernelFactory,
    KernelHandshake, KernelStatus, ValueProfile,
};
use scires_core::{DomainEvent, EventBus, Language, Variable};
use std::collections::HashMap;
use std::sync::Arc;

struct RunningKernel {
    kernel: Box<dyn Kernel>,
    handshake: KernelHandshake,
    negotiated_profiles: Vec<ValueProfile>,
}

fn publish_status(events: &Arc<dyn EventBus>, language: &Language, status: KernelStatus) {
    events.publish(DomainEvent::KernelStatus {
        language: language.clone(),
        status: status.as_str().into(),
    });
}

/// Owns factories and running kernels; routes requests by language.
/// Kernels are started lazily on first use and validated against the versioned
/// SciRes kernel handshake before accepting requests.
pub struct KernelManager {
    factories: HashMap<Language, Arc<dyn KernelFactory>>,
    running: HashMap<Language, RunningKernel>,
    supported_profiles: Vec<ValueProfile>,
    events: Arc<dyn EventBus>,
}

impl KernelManager {
    /// Create an empty manager supporting the v1 SciRes Arrow value profile.
    pub fn new(events: Arc<dyn EventBus>) -> Self {
        Self {
            factories: HashMap::new(),
            running: HashMap::new(),
            supported_profiles: vec![ValueProfile::scires_v1()],
            events,
        }
    }

    /// Override the value profiles supported by the host during negotiation.
    pub fn with_supported_value_profiles(mut self, profiles: Vec<ValueProfile>) -> Self {
        self.supported_profiles = profiles;
        self
    }

    /// Register (or replace) a factory. Returns the previous one, if any.
    pub fn register(&mut self, factory: Arc<dyn KernelFactory>) -> Option<Arc<dyn KernelFactory>> {
        self.factories.insert(factory.language(), factory)
    }

    /// Unregister a factory, shutting down its running kernel.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Self::shutdown`].
    pub fn unregister(&mut self, language: &Language) -> Result<(), KernelError> {
        self.shutdown(language)?;
        self.factories.remove(language);
        Ok(())
    }

    /// Languages with a registered factory, sorted.
    pub fn available(&self) -> Vec<Language> {
        let mut v: Vec<_> = self.factories.keys().cloned().collect();
        v.sort();
        v
    }

    /// Status of the kernel for `language`.
    pub fn status(&self, language: &Language) -> KernelStatus {
        self.running
            .get(language)
            .map_or(KernelStatus::NotStarted, |rk| rk.kernel.status())
    }

    /// Validated handshake of a running kernel, if started.
    pub fn handshake(&self, language: &Language) -> Option<&KernelHandshake> {
        self.running.get(language).map(|rk| &rk.handshake)
    }

    /// Value profiles negotiated between the host and a running kernel.
    pub fn negotiated_value_profiles(&self, language: &Language) -> Vec<ValueProfile> {
        self.running
            .get(language)
            .map(|rk| rk.negotiated_profiles.clone())
            .unwrap_or_default()
    }

    /// Thread-safe control handle for out-of-band operations (such as concurrent interrupt).
    pub fn control(&self, language: &Language) -> Option<Arc<dyn KernelControl>> {
        self.running
            .get(language)
            .and_then(|rk| rk.kernel.control())
    }

    fn running_mut(&mut self, language: &Language) -> Result<&mut RunningKernel, KernelError> {
        if !self.running.contains_key(language) {
            let factory = self
                .factories
                .get(language)
                .ok_or_else(|| KernelError::Unsupported(language.clone()))?;
            self.publish_status(language, KernelStatus::Starting);
            let mut kernel = match factory.create() {
                Ok(kernel) => kernel,
                Err(err) => {
                    self.publish_status(language, KernelStatus::Failed);
                    return Err(err);
                }
            };
            if let Err(err) = kernel.start() {
                self.publish_status(language, KernelStatus::Failed);
                return Err(err);
            }
            self.publish_status(language, KernelStatus::Handshaking);
            let handshake = kernel.handshake();
            if let Err(err) = handshake.validate() {
                let _ = kernel.shutdown();
                self.publish_status(language, KernelStatus::Failed);
                return Err(err);
            }
            let negotiated_profiles = handshake.negotiate_profiles(&self.supported_profiles);
            self.running.insert(
                language.clone(),
                RunningKernel {
                    kernel,
                    handshake,
                    negotiated_profiles,
                },
            );
            self.publish_status(language, KernelStatus::Idle);
        }
        Ok(self.running.get_mut(language).expect("just inserted"))
    }

    fn publish_status(&self, language: &Language, status: KernelStatus) {
        publish_status(&self.events, language, status);
    }

    /// Execute code in the kernel for `language`, starting it if needed.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::Unsupported`] if no factory is registered for `language`,
    /// [`KernelError::Startup`] or [`KernelError::Handshake`] if lazy startup fails,
    /// [`KernelError::CapabilityUnavailable`] if `execute` is not advertised, or
    /// execution/transport errors from the underlying kernel.
    pub fn execute(
        &mut self,
        language: &Language,
        req: &ExecuteRequest,
    ) -> Result<ExecuteResult, KernelError> {
        let rk = self.running_mut(language)?;
        if !rk.handshake.capabilities.execute {
            return Err(KernelError::CapabilityUnavailable("execute"));
        }
        rk.kernel.execute(req)
    }

    /// Variables of a language's kernel.
    ///
    /// # Errors
    ///
    /// Propagates startup, handshake, or kernel query errors.
    pub fn variables(&mut self, language: &Language) -> Result<Vec<Variable>, KernelError> {
        self.running_mut(language)?.kernel.variables()
    }

    /// Export a named variable from a kernel using a negotiated value profile.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::UnsupportedValueProfile`] if `profile` was not negotiated
    /// with the kernel, or propagates startup/transfer errors.
    pub fn export_value(
        &mut self,
        language: &Language,
        name: &str,
        profile: &ValueProfile,
    ) -> Result<Vec<u8>, KernelError> {
        let rk = self.running_mut(language)?;
        if !rk.negotiated_profiles.contains(profile) {
            return Err(KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            });
        }
        rk.kernel.export_value(name, profile)
    }

    /// Import a named value into a kernel using a negotiated value profile.
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::UnsupportedValueProfile`] if `profile` was not negotiated
    /// with the kernel, or propagates startup/transfer errors.
    pub fn import_value(
        &mut self,
        language: &Language,
        name: &str,
        profile: &ValueProfile,
        payload: &[u8],
    ) -> Result<(), KernelError> {
        let rk = self.running_mut(language)?;
        if !rk.negotiated_profiles.contains(profile) {
            return Err(KernelError::UnsupportedValueProfile {
                id: profile.id.clone(),
                version: profile.version,
            });
        }
        rk.kernel.import_value(name, profile, payload)
    }

    /// Interrupt a running kernel (no-op if not started).
    ///
    /// Uses the kernel's thread-safe [`KernelControl`] handle when available,
    /// falling back to [`Kernel::interrupt`].
    ///
    /// # Errors
    ///
    /// Returns [`KernelError::CapabilityUnavailable`] if the kernel does not advertise
    /// interrupt support, or propagates transport errors.
    pub fn interrupt(&mut self, language: &Language) -> Result<(), KernelError> {
        let Some(rk) = self.running.get_mut(language) else {
            return Ok(());
        };
        if !rk.handshake.capabilities.interrupt {
            return Err(KernelError::CapabilityUnavailable("interrupt"));
        }
        publish_status(&self.events, language, KernelStatus::Interrupting);
        let result = if let Some(control) = rk.kernel.control() {
            control.interrupt()
        } else {
            rk.kernel.interrupt()
        };
        let status = rk.kernel.status();
        publish_status(&self.events, language, status);
        result
    }

    /// Shut down a kernel (no-op if not started).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Kernel::shutdown`].
    pub fn shutdown(&mut self, language: &Language) -> Result<(), KernelError> {
        if let Some(mut rk) = self.running.remove(language) {
            rk.kernel.shutdown()?;
            publish_status(&self.events, language, KernelStatus::Stopped);
        }
        Ok(())
    }

    /// Restart a kernel: shutdown, clear its workspace namespace, start fresh,
    /// and re-validate its handshake.
    ///
    /// # Errors
    ///
    /// Propagates errors from restart, startup, or handshake validation.
    pub fn restart(&mut self, language: &Language) -> Result<(), KernelError> {
        if let Some(rk) = self.running.get_mut(language) {
            if !rk.handshake.capabilities.restart {
                return Err(KernelError::CapabilityUnavailable("restart"));
            }
            publish_status(&self.events, language, KernelStatus::Restarting);
            if let Err(err) = rk.kernel.restart() {
                publish_status(&self.events, language, KernelStatus::Failed);
                return Err(err);
            }
            publish_status(&self.events, language, KernelStatus::Handshaking);
            let handshake = rk.kernel.handshake();
            if let Err(err) = handshake.validate() {
                let _ = rk.kernel.shutdown();
                self.running.remove(language);
                publish_status(&self.events, language, KernelStatus::Failed);
                return Err(err);
            }
            rk.negotiated_profiles = handshake.negotiate_profiles(&self.supported_profiles);
            rk.handshake = handshake;
            publish_status(&self.events, language, KernelStatus::Idle);
        } else {
            self.running_mut(language).map(|_| ())?;
        }
        self.events
            .publish(DomainEvent::WorkspaceCleared(Some(language.clone())));
        Ok(())
    }

    /// Shut down every kernel.
    ///
    /// # Errors
    ///
    /// Returns the first error encountered while shutting down active kernels.
    pub fn shutdown_all(&mut self) -> Result<(), KernelError> {
        let langs: Vec<_> = self.running.keys().cloned().collect();
        langs.iter().try_for_each(|l| self.shutdown(l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{EchoKernel, EchoKernelFactory};
    use crate::{KernelCapabilities, KernelInfo, Output, RequestId, VALUE_PROFILE_ID};
    use scires_core::InMemoryEventBus;
    use std::sync::Mutex;

    fn manager() -> KernelManager {
        let mut m = KernelManager::new(Arc::new(InMemoryEventBus::new()));
        m.register(Arc::new(EchoKernelFactory::new(Language::Julia)));
        m
    }

    struct CustomHandshakeFactory {
        handshake: KernelHandshake,
    }

    struct CustomHandshakeKernel {
        status: KernelStatus,
        handshake: KernelHandshake,
    }

    impl Kernel for CustomHandshakeKernel {
        fn info(&self) -> KernelInfo {
            self.handshake.info.clone()
        }
        fn status(&self) -> KernelStatus {
            self.status
        }
        fn start(&mut self) -> Result<(), KernelError> {
            self.status = KernelStatus::Idle;
            Ok(())
        }
        fn execute(&mut self, req: &ExecuteRequest) -> Result<ExecuteResult, KernelError> {
            Ok(ExecuteResult {
                request_id: req.request_id.clone(),
                execution_count: 1,
                outputs: vec![],
            })
        }
        fn interrupt(&mut self) -> Result<(), KernelError> {
            Err(KernelError::CapabilityUnavailable("interrupt"))
        }
        fn variables(&mut self) -> Result<Vec<Variable>, KernelError> {
            Ok(vec![])
        }
        fn shutdown(&mut self) -> Result<(), KernelError> {
            self.status = KernelStatus::Stopped;
            Ok(())
        }
        fn handshake(&self) -> KernelHandshake {
            self.handshake.clone()
        }
    }

    impl KernelFactory for CustomHandshakeFactory {
        fn language(&self) -> Language {
            self.handshake.info.language.clone()
        }
        fn create(&self) -> Result<Box<dyn Kernel>, KernelError> {
            Ok(Box::new(CustomHandshakeKernel {
                status: KernelStatus::NotStarted,
                handshake: self.handshake.clone(),
            }))
        }
    }

    #[test]
    fn lazily_starts_validates_handshake_and_correlates_requests() {
        let bus = Arc::new(InMemoryEventBus::new());
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&statuses);
        bus.subscribe(Box::new(move |event| {
            if let DomainEvent::KernelStatus { status, .. } = event {
                captured.lock().unwrap().push(status.clone());
            }
        }));

        let mut m = KernelManager::new(bus);
        m.register(Arc::new(EchoKernelFactory::new(Language::Julia)));
        assert_eq!(m.status(&Language::Julia), KernelStatus::NotStarted);
        assert!(m.handshake(&Language::Julia).is_none());

        let req = ExecuteRequest::new("1+1").with_request_id(RequestId::new("req-test-1"));
        let r = m.execute(&Language::Julia, &req).unwrap();
        assert_eq!(r.request_id, RequestId::new("req-test-1"));
        assert_eq!(r.execution_count, 1);
        assert_eq!(r.outputs, vec![Output::Stdout("1+1".into())]);
        assert_eq!(m.status(&Language::Julia), KernelStatus::Idle);
        assert_eq!(
            m.negotiated_value_profiles(&Language::Julia),
            vec![ValueProfile::scires_v1()]
        );
        assert_eq!(
            *statuses.lock().unwrap(),
            vec!["starting", "handshaking", "idle"]
        );
    }

    #[test]
    fn rejects_unsupported_handshake_version() {
        let mut bad = KernelHandshake::new(
            KernelInfo {
                language: Language::Julia,
                implementation: "future".into(),
                language_version: "2.0".into(),
            },
            KernelCapabilities::default(),
        );
        bad.version = 99;

        let mut m = KernelManager::new(Arc::new(InMemoryEventBus::new()));
        m.register(Arc::new(CustomHandshakeFactory { handshake: bad }));
        let err = m
            .execute(&Language::Julia, &ExecuteRequest::new("1"))
            .unwrap_err();
        assert!(matches!(err, KernelError::Handshake(_)));
        assert_eq!(m.status(&Language::Julia), KernelStatus::NotStarted);
    }

    #[test]
    fn unsupported_language() {
        let mut m = manager();
        let e = m
            .execute(&Language::R, &ExecuteRequest::new("x"))
            .unwrap_err();
        assert_eq!(e, KernelError::Unsupported(Language::R));
    }

    #[test]
    fn restart_resets_counter_and_clears_transferred_values() {
        let mut m = manager();
        let profile = ValueProfile::scires_v1();
        m.import_value(&Language::Julia, "arr", &profile, b"arrow-bytes")
            .unwrap();
        assert_eq!(
            m.export_value(&Language::Julia, "arr", &profile).unwrap(),
            b"arrow-bytes"
        );

        m.execute(&Language::Julia, &ExecuteRequest::new("a"))
            .unwrap();
        m.restart(&Language::Julia).unwrap();
        let r = m
            .execute(&Language::Julia, &ExecuteRequest::new("b"))
            .unwrap();
        assert_eq!(r.execution_count, 1);
        assert!(matches!(
            m.export_value(&Language::Julia, "arr", &profile),
            Err(KernelError::Transfer(_))
        ));
    }

    #[test]
    fn rejects_unnegotiated_value_profile() {
        let mut m = manager();
        let v2 = ValueProfile::new(VALUE_PROFILE_ID, 2);
        assert_eq!(
            m.import_value(&Language::Julia, "x", &v2, b"bytes"),
            Err(KernelError::UnsupportedValueProfile {
                id: VALUE_PROFILE_ID.into(),
                version: 2,
            })
        );
    }

    #[test]
    fn concurrent_control_interrupt_signals_kernel() {
        let mut m = manager();
        m.execute(&Language::Julia, &ExecuteRequest::new("warmup"))
            .unwrap();
        let control = m
            .control(&Language::Julia)
            .expect("echo kernel exposes control handle");
        let handle = std::thread::spawn(move || control.interrupt());
        handle.join().unwrap().unwrap();

        let err = m
            .execute(&Language::Julia, &ExecuteRequest::new("next"))
            .unwrap_err();
        assert_eq!(err, KernelError::Interrupted);
    }

    #[test]
    fn interrupt_unavailable_when_not_advertised() {
        let no_interrupt = KernelHandshake::new(
            KernelInfo {
                language: Language::Python,
                implementation: "sync".into(),
                language_version: "3.12".into(),
            },
            KernelCapabilities {
                execute: true,
                interrupt: false,
                shutdown: true,
                restart: true,
            },
        );
        let mut m = KernelManager::new(Arc::new(InMemoryEventBus::new()));
        m.register(Arc::new(CustomHandshakeFactory {
            handshake: no_interrupt,
        }));
        m.execute(&Language::Python, &ExecuteRequest::new("1"))
            .unwrap();
        assert_eq!(
            m.interrupt(&Language::Python),
            Err(KernelError::CapabilityUnavailable("interrupt"))
        );
    }

    #[test]
    fn echo_kernel_default_control_works_directly() {
        let mut kernel = EchoKernel::new(Language::Julia);
        kernel.start().unwrap();
        let control = kernel.control().unwrap();
        control.interrupt().unwrap();
        assert_eq!(
            kernel.execute(&ExecuteRequest::new("1")),
            Err(KernelError::Interrupted)
        );
    }
}
