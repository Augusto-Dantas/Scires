use crate::{ExecuteRequest, ExecuteResult, Kernel, KernelError, KernelFactory, KernelStatus};
use scires_core::{DomainEvent, EventBus, Language, Variable};
use std::collections::HashMap;
use std::sync::Arc;

/// Owns factories and running kernels; routes requests by language.
/// Kernels are started lazily on first use.
pub struct KernelManager {
    factories: HashMap<Language, Arc<dyn KernelFactory>>,
    running: HashMap<Language, Box<dyn Kernel>>,
    events: Arc<dyn EventBus>,
}

impl KernelManager {
    /// Create an empty manager.
    pub fn new(events: Arc<dyn EventBus>) -> Self {
        Self {
            factories: HashMap::new(),
            running: HashMap::new(),
            events,
        }
    }

    /// Register (or replace) a factory. Returns the previous one, if any.
    pub fn register(&mut self, factory: Arc<dyn KernelFactory>) -> Option<Arc<dyn KernelFactory>> {
        self.factories.insert(factory.language(), factory)
    }

    /// Unregister a factory, shutting down its running kernel.
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
            .map_or(KernelStatus::NotStarted, |k| k.status())
    }

    fn kernel(&mut self, language: &Language) -> Result<&mut Box<dyn Kernel>, KernelError> {
        if !self.running.contains_key(language) {
            let factory = self
                .factories
                .get(language)
                .ok_or_else(|| KernelError::Unsupported(language.clone()))?;
            let mut k = factory.create()?;
            k.start()?;
            self.running.insert(language.clone(), k);
            self.publish_status(language, "idle");
        }
        Ok(self.running.get_mut(language).expect("just inserted"))
    }

    fn publish_status(&self, language: &Language, status: &str) {
        self.events.publish(DomainEvent::KernelStatus {
            language: language.clone(),
            status: status.into(),
        });
    }

    /// Execute code in the kernel for `language`, starting it if needed.
    pub fn execute(
        &mut self,
        language: &Language,
        req: &ExecuteRequest,
    ) -> Result<ExecuteResult, KernelError> {
        self.kernel(language)?.execute(req)
    }

    /// Variables of a language's kernel.
    pub fn variables(&mut self, language: &Language) -> Result<Vec<Variable>, KernelError> {
        self.kernel(language)?.variables()
    }

    /// Interrupt a running kernel (no-op if not started).
    pub fn interrupt(&mut self, language: &Language) -> Result<(), KernelError> {
        match self.running.get_mut(language) {
            Some(k) => k.interrupt(),
            None => Ok(()),
        }
    }

    /// Shut down a kernel (no-op if not started).
    pub fn shutdown(&mut self, language: &Language) -> Result<(), KernelError> {
        if let Some(mut k) = self.running.remove(language) {
            k.shutdown()?;
            self.publish_status(language, "dead");
        }
        Ok(())
    }

    /// Restart a kernel: shutdown, clear its workspace namespace, start fresh.
    pub fn restart(&mut self, language: &Language) -> Result<(), KernelError> {
        self.shutdown(language)?;
        self.events
            .publish(DomainEvent::WorkspaceCleared(Some(language.clone())));
        self.kernel(language).map(|_| ())
    }

    /// Shut down every kernel.
    pub fn shutdown_all(&mut self) -> Result<(), KernelError> {
        let langs: Vec<_> = self.running.keys().cloned().collect();
        langs.iter().try_for_each(|l| self.shutdown(l))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::EchoKernelFactory;
    use crate::Output;
    use scires_core::InMemoryEventBus;

    fn manager() -> KernelManager {
        let mut m = KernelManager::new(Arc::new(InMemoryEventBus::new()));
        m.register(Arc::new(EchoKernelFactory::new(Language::Julia)));
        m
    }

    #[test]
    fn lazily_starts_and_executes() {
        let mut m = manager();
        assert_eq!(m.status(&Language::Julia), KernelStatus::NotStarted);
        let r = m.execute(&Language::Julia, &ExecuteRequest::new("1+1")).unwrap();
        assert_eq!(r.execution_count, 1);
        assert_eq!(r.outputs, vec![Output::Stdout("1+1".into())]);
        assert_eq!(m.status(&Language::Julia), KernelStatus::Idle);
    }

    #[test]
    fn unsupported_language() {
        let mut m = manager();
        let e = m.execute(&Language::R, &ExecuteRequest::new("x")).unwrap_err();
        assert_eq!(e, KernelError::Unsupported(Language::R));
    }

    #[test]
    fn restart_resets_counter() {
        let mut m = manager();
        m.execute(&Language::Julia, &ExecuteRequest::new("a")).unwrap();
        m.restart(&Language::Julia).unwrap();
        let r = m.execute(&Language::Julia, &ExecuteRequest::new("b")).unwrap();
        assert_eq!(r.execution_count, 1);
    }
}
