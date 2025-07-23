use pyo3::prelude::*;
use pythonize::pythonize;

/// Convert a serde_json::Value to a Python object
pub fn json_to_pyobject(py: Python, value: &serde_json::Value) -> PyObject {
    // pythonize handles all the conversion automatically!
    pythonize(py, value).unwrap().into()
}
