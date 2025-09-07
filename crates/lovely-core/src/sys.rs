use std::ptr;
use std::sync::OnceLock;
use std::slice;
use std::ffi::{c_char, c_int, c_void, CString};
use std::collections::VecDeque;

use itertools::Itertools;
use libloading::Library;
use log::info;

pub static LUA: OnceLock<LuaLib> = OnceLock::new();

pub type LuaState = c_void;

pub const LUA_GLOBALSINDEX: c_int = -10002;
pub const LUA_TNIL: c_int = 0;
pub const LUA_TBOOLEAN: c_int = 1;

macro_rules! c {
    ($s:expr) => {
        concat!($s, "\0").as_ptr() as *const c_char
    };
}

macro_rules! generate {
    ($libname:ident {
        $(
            $vis:vis unsafe extern "C" fn $method:ident($($arg:ident: $ty:ty),*) $(-> $ret:ty)?;
        )*
    }) => {
        pub struct $libname {
            $(
                $vis $method: unsafe extern "C" fn($($arg: $ty),*) $(-> $ret)?,
            )*
        }

        $(
            /// # Safety
            $vis unsafe extern "C" fn $method($($arg: $ty),*) $(-> $ret)? {
                let lua = LUA.get().unwrap_or_else(|| panic!("Failed to access Lua lib defs"));
                (lua.$method)($($arg),*)
            }
        )*
    };
}

generate! (LuaLib {
    pub unsafe extern "C" fn lua_call(state: *mut LuaState, nargs: c_int, nresults: c_int);
    pub unsafe extern "C" fn lua_pcall(state: *mut LuaState, nargs: c_int, nresults: c_int, errfunc: c_int) -> c_int;
    pub unsafe extern "C" fn lua_getfield(state: *mut LuaState, index: c_int, k: *const c_char);
    pub unsafe extern "C" fn lua_setfield(state: *mut LuaState, index: c_int, k: *const c_char);
    pub unsafe extern "C" fn lua_gettop(state: *mut LuaState) -> c_int;
    pub unsafe extern "C" fn lua_settop(state: *mut LuaState, index: c_int);
    pub unsafe extern "C" fn lua_pushvalue(state: *mut LuaState, index: c_int);
    pub unsafe extern "C" fn lua_pushcclosure(state: *mut LuaState, f: unsafe extern "C" fn(*mut LuaState) -> c_int, n: c_int);
    pub unsafe extern "C" fn lua_tolstring(state: *mut LuaState, index: c_int, len: *mut usize) -> *const c_char;
    // Add the missing functions from the original code
    pub unsafe extern "C" fn lua_toboolean(state: *mut LuaState, index: c_int) -> c_int;
    pub unsafe extern "C" fn lua_topointer(state: *mut LuaState, index: c_int) -> *const c_void;
    pub unsafe extern "C" fn lua_type(state: *mut LuaState, index: c_int) -> c_int;
    pub unsafe extern "C" fn lua_typename(state: *mut LuaState, tp: c_int) -> *const c_char;
    pub unsafe extern "C" fn lua_isstring(state: *mut LuaState, index: c_int) -> c_int;
});

impl LuaLib {
    /// Construct a LuaLib from a loaded library.
    /// # Safety
    /// The library must define Lua symbols.
    pub unsafe fn from_library(library: &Library) -> Self {
        LuaLib {
            lua_call: *library.get(b"lua_call\0").unwrap(),
            lua_pcall: *library.get(b"lua_pcall\0").unwrap(),
            lua_getfield: *library.get(b"lua_getfield\0").unwrap(),
            lua_setfield: *library.get(b"lua_setfield\0").unwrap(),
            lua_gettop: *library.get(b"lua_gettop\0").unwrap(),
            lua_settop: *library.get(b"lua_settop\0").unwrap(),
            lua_pushvalue: *library.get(b"lua_pushvalue\0").unwrap(),
            lua_pushcclosure: *library.get(b"lua_pushcclosure\0").unwrap(),
            lua_tolstring: *library.get(b"lua_tolstring\0").unwrap(),
            lua_toboolean: *library.get(b"lua_toboolean\0").unwrap(),
            lua_topointer: *library.get(b"lua_topointer\0").unwrap(),
            lua_type: *library.get(b"lua_type\0").unwrap(),
            lua_typename: *library.get(b"lua_typename\0").unwrap(),
            lua_isstring: *library.get(b"lua_isstring\0").unwrap(),
        }
    }
}

/// Initialize the Lua library based on the target OS
pub fn init_lua_library() -> Result<(), Box<dyn std::error::Error>> {
    let library = unsafe {
        #[cfg(target_os = "windows")]
        Library::new("lua51.dll")?;
        
        #[cfg(target_os = "macos")]
        Library::new("../Frameworks/Lua.framework/Versions/A/Lua")?;
        
        #[cfg(target_os = "linux")]
        Library::new("libluajit-5.1.so.2")?;
        
        #[cfg(target_os = "android")]
        Library::new("liblove.so")?;
        
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "android")))]
        Library::new("liblua5.1.so")?;
    };
    
    let lua_lib = unsafe { LuaLib::from_library(&library) };
    LUA.set(lua_lib).map_err(|_| "LUA already initialized")?;
    
    Ok(())
}

/// Load the provided buffer as a lua module with the specified name.
/// # Safety
/// Makes a lot of FFI calls, mutates internal C lua state.
pub unsafe fn load_module<F: Fn(*mut LuaState, *const u8, usize, *const u8, *const u8) -> u32>(
    state: *mut LuaState,
    name: &str,
    buffer: &str,
    lual_loadbufferx: &F,
) {
    let buf_cstr = CString::new(buffer).unwrap();
    let buf_len = buf_cstr.as_bytes().len();

    let p_name = format!("@{name}");
    let p_name_cstr = CString::new(p_name).unwrap();

    // Push the global package.preload table onto the top of the stack, saving its index.
    let stack_top = lua_gettop(state);
    lua_getfield(state, LUA_GLOBALSINDEX, c!("package"));
    lua_getfield(state, -1, c!("preload"));

    // This is the index of the `package.preload` table.
    let field_index = lua_gettop(state);

    // Load the buffer and execute it via lua_pcall, pushing the result to the top of the stack.
    lual_loadbufferx(
        state,
        buf_cstr.as_ptr() as *const u8,
        buf_len,
        p_name_cstr.as_ptr() as *const u8,
        ptr::null(),
    );

    let lua_pcall_return = lua_pcall(state, 0, 1, 0);
    if lua_pcall_return == 0 {
        lua_pushcclosure(state, lua_identity_closure, 1);
        // Insert wrapped pcall results onto the package.preload table.
        let module_cstr = CString::new(name).unwrap();
        lua_setfield(state, field_index, module_cstr.as_ptr());
    }

    lua_settop(state, stack_top);
}

/// An override print function, copied piecemeal from the Lua 5.1 source, but in Rust.
/// # Safety
/// Native lua API access. It's unsafe, it's unchecked, it will probably eat your firstborn.
pub unsafe extern "C" fn override_print(state: *mut LuaState) -> c_int {
    let argc = lua_gettop(state);
    let mut out = VecDeque::new();

    for i in 1..=argc {
        // We call Lua's builtin tostring function because we don't have access to the 5.3 luaL_tolstring
        // helper function. It's not pretty, but it works.
        lua_getfield(state, LUA_GLOBALSINDEX, c!("tostring"));
        lua_pushvalue(state, i);
        lua_call(state, 1, 1);

        let mut str_len = 0usize;
        let arg_str = lua_tolstring(state, -1, &mut str_len);

        let str_buf = slice::from_raw_parts(arg_str as *const u8, str_len);
        let arg_str = String::from_utf8_lossy(str_buf).to_string();

        out.push_back(arg_str);
        lua_settop(state, -2); // Remove the tostring result, keep original args
    }

    let msg = out.into_iter().join("\t");
    info!("[G] {msg}");

    0
}

/// A function, which as a Lua closure, returns the first upvalue. This lets it
/// be used to wrap lua values into a closure which returns that value.
/// # Safety
/// Makes some FFI calls, mutates internal C lua state.
pub unsafe extern "C" fn lua_identity_closure(state: *mut LuaState) -> c_int {
    // LUA_GLOBALSINDEX - 1 is where the first upvalue is located
    lua_pushvalue(state, LUA_GLOBALSINDEX - 1);
    // We just return that value
    1
}