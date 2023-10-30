use notify::{Watcher, RecursiveMode};
use xplm::plugin::internal;
use xplm::{plugin::{Plugin, PluginInfo, management::this_plugin}, flight_loop::{FlightLoop, LoopState}};

use dlopen::wrapper::{WrapperApi, Container};


struct ReloadPlugin {
    active: Option<ReloadPluginActive>,
}


#[derive(Debug, thiserror::Error)]
enum ReloadError {
    #[error(transparent)]
    PluginLoadError(#[from] PluginLoadError),
}

impl ReloadPlugin {
    unsafe fn on_message(
        &self,
        from: raw::c_int,
        message: raw::c_int,
        param: *mut raw::c_void,
    ) {
        if let Some(active) = &self.active {
            if let Some(loaded) = &*active.loaded.borrow_mut() {
                loaded.on_message(from, message, param)
            }
        }
    }
}

impl Plugin for ReloadPlugin {
    type Error = ReloadError;

    fn start() -> Result<Self, Self::Error> {
        let active = None;
        Ok(Self { active })
    }

    fn enable(&mut self) -> Result<(), Self::Error> {
        if let Some(_) = self.active.replace(ReloadPluginActive::new()?) {
            xplm::debug("reload(warn) tried to enable already active plugin\n");
        } else {
            xplm::debug("reload(info) activating loaded plugin\n");
        }
        Ok(())
    }

    fn disable(&mut self) {
        if self.active.take().is_none() {
            xplm::debug("reload(info) no plugin loaded: noting to deactivate\n");
        } else {
            xplm::debug("reload(info) deactivating loaded plugin\n");
        }
    }

    fn info(&self) -> PluginInfo {
        PluginInfo {
            name: "xplane-reload".to_string(),
            signature: "dev.dvmujic.reload".to_string(),
            description: "a plugin to automaticly reload its child".to_string(),
        }
    }
}


struct ReloadPluginActive {
    loaded: Rc<RefCell<Option<LoadedPlugin>>>,
    _watcher: Box<dyn Watcher>,
    _reload_loop: FlightLoop,
}

impl ReloadPluginActive {
    fn new() -> Result<Self, ReloadError> {
        let loaded = Rc::new(RefCell::new(Some(LoadedPlugin::new()?)));
        let (tx, rx) = mpsc::channel::<()>();

        let mut _reload_loop = FlightLoop::new({
            let loaded = Rc::clone(&loaded);
            move |_state: &mut LoopState| {
                while let Ok(_) = rx.try_recv() {
                    Self::reload_plugin(&loaded)
                }
            }
        });
        _reload_loop.schedule_immediate();

        let (watcher_tx, watcher_rx) = mpsc::channel();

        let mut _watcher = Box::new(notify::recommended_watcher(watcher_tx).expect("could not create watcher"));

        _watcher.watch(
            &LoadedPlugin::path().with_file_name(LoadedPlugin::NEW_NAME),
            RecursiveMode::NonRecursive
        ).expect("could not start watcher");

        std::thread::spawn(move || {
            for res in watcher_rx {
                match res {
                    Ok(ev) => {
                        xplm::debug(format!("reload(info) sending reload command (ev: {ev:?})"));
                        tx.send(()).expect("could not sent reload command");
                    },
                    Err(e) => {
                        xplm::debug(format!("reload(error) watch error: {e}"));
                    },
                }
            }
        });

        Ok(Self { loaded, _reload_loop, _watcher })
    }

    /// should only be called from main xplane thread
    fn reload_plugin(loaded: &Rc<RefCell<Option<LoadedPlugin>>>) {
        let mut loaded = loaded.borrow_mut();
        if loaded.take().is_some() {

        } else {  }

        *loaded = match LoadedPlugin::new() {
            Ok(v) => Some(v),
            Err(err) => {
                xplm::debug(format!("could not reload plugin: {err}\n"));
                None
            },
        }
    }
}



use core::ffi::FromBytesUntilNulError;
use std::{os::raw, ffi::CStr, str::Utf8Error, rc::Rc, cell::RefCell, sync::mpsc, path::PathBuf};

#[derive(dlopen_derive::WrapperApi)]
#[allow(nonstandard_style)]
struct Api {
    // #[dlopen_name("XPluginStart")]
    XPluginStart: unsafe extern "C" fn(
        name: *mut raw::c_char,
        signature: *mut raw::c_char,
        description: *mut raw::c_char,
    ) -> raw::c_int,

    // #[dlopen_name = "XPluginStop"]
    XPluginStop: unsafe extern "C" fn(),

    // #[dlopen_name = "XPluginEnable"]
    XPluginEnable: unsafe extern "C" fn() -> raw::c_int,

    // #[dlopen_name = "XPluginDisable"]
    XPluginDisable: unsafe extern "C" fn(),

    // #[dlopen_name = "XPluginReceiveMessage"]
    XPluginReceiveMessage: unsafe extern "C" fn(
        from: raw::c_int,
        msg: raw::c_int,
        param: *mut raw::c_void,
    ),
}

#[derive(Debug, thiserror::Error)]
enum PluginLoadError {
    #[error(transparent)]
    Dlopen(#[from] dlopen::Error),

    #[error("XPluginStart returned false")]
    StartFailed,

    #[error("XPluginEnable returned false")]
    EnableFailed,

    #[error(transparent)]
    InvalidString(#[from] FromBytesUntilNulError),

    #[error(transparent)]
    Utf8Error(#[from] Utf8Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

struct LoadedPlugin {
    container: Container<Api>,
}

impl LoadedPlugin {
    const LOAD_NAME: &str = "_plugin.reload";
    const NEW_NAME: &str = "plugin.reload";

    fn path() -> PathBuf {
        let plugin = this_plugin();
        let plugin_path = plugin.path();
        plugin_path.to_owned()
    }

    fn new() -> Result<Self, PluginLoadError> {
        let path = Self::path();
        let new_path = path.with_file_name(LoadedPlugin::NEW_NAME);
        let loaded_path = path.with_file_name(LoadedPlugin::LOAD_NAME);
        xplm::debug(format!("path: {path:?}, file: {new_path:?}; {loaded_path:?}\n"));

        std::fs::copy(&new_path, &loaded_path)?;

        let container: Container<Api> = unsafe {
            Container::load(&loaded_path)?
        };

        let mut name = [0u8; 256];
        let mut signature = [0u8; 256];
        let mut description = [0u8; 256];

        unsafe {
            if container.XPluginStart(
                name.as_mut_ptr() as _,
                signature.as_mut_ptr() as _,
                description.as_mut_ptr() as _,
            ) == 0 { Err(PluginLoadError::StartFailed)? }
        }

        let name = CStr::from_bytes_until_nul(&name[..])?.to_str()?;
        let signature = CStr::from_bytes_until_nul(&signature[..])?.to_str()?;
        let description = CStr::from_bytes_until_nul(&description[..])?.to_str()?;

        xplm::debug(format!("reload(info) loaded plugin '{name}' with signature '{signature}': {description}\n"));

        if unsafe { container.XPluginEnable() == 0 } { Err(PluginLoadError::EnableFailed)? }

        Ok(Self { container })
    }

    unsafe fn on_message(&self, from: raw::c_int, msg: raw::c_int, param: *mut raw::c_void) {
        self.container.XPluginReceiveMessage(from, msg, param)
    }
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        unsafe {
            xplm::debug("reload(info) deactivating loaded plugin\n");
            self.container.XPluginDisable();
            xplm::debug("reload(info) stopping loaded plugin\n");
            self.container.XPluginStop();
            xplm::debug("reload(info) stopped loaded plugin\n");
        };
    }
}


// internal plugin stuff because ReceiveMessage was not supported

static mut PLUGIN: internal::PluginData<ReloadPlugin> = internal::PluginData {
    plugin: 0 as *mut _,
    panicked: false,
};

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn XPluginStart(
    name: *mut raw::c_char,
    signature: *mut raw::c_char,
    description: *mut raw::c_char
) -> raw::c_int {
  internal::xplugin_start(&mut PLUGIN,name,signature,description)
}
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn XPluginStop(){ internal::xplugin_stop(&mut PLUGIN) }
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn XPluginEnable() ->  raw::c_int { internal::xplugin_enable(&mut PLUGIN) }
#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn XPluginDisable(){ internal::xplugin_disable(&mut PLUGIN) }

#[allow(non_snake_case)]
#[no_mangle]
pub unsafe extern "C" fn XPluginReceiveMessage(
    from: raw::c_int,
    message: raw::c_int,
    param: *mut raw::c_void,
){
    (*PLUGIN.plugin).on_message(from, message, param)
}

