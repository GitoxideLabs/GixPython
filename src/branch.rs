//! Native branch configuration and refspec-based upstream/tracking queries.

use pyo3::prelude::*;

use crate::repository::Repository;

#[pymethods]
impl Repository {
    fn branch_names(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        self.handle.run(py, |repo| {
            Ok(repo.branch_names().into_iter().map(str::to_owned).collect())
        })
    }
}

#[cfg(feature = "network")]
mod remotes {
    use crate::{
        error::to_py,
        network::{Direction, Remote, make_remote},
        references::{Head, Reference},
        repository::Repository,
        types::bytes,
    };
    use gix::{bstr::ByteSlice, prelude::ReferenceExt};
    use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyBytes};
    use std::sync::{Arc, Mutex};

    fn full_name(name: &Bound<'_, PyAny>) -> PyResult<gix::refs::FullName> {
        gix::refs::FullName::try_from(gix::bstr::BString::from(bytes(name)?)).map_err(to_py)
    }

    #[pymethods]
    impl Repository {
        fn branch_remote_ref_name<'py>(
            &self,
            py: Python<'py>,
            name: &Bound<'_, PyAny>,
            direction: Direction,
        ) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let name = full_name(name)?;
            let result = self.handle.run(py, move |repo| {
                repo.branch_remote_ref_name(name.as_ref(), direction.inner)
                    .transpose()
                    .map_err(to_py)
            })?;
            Ok(result.map(|value| PyBytes::new(py, value.as_bstr())))
        }
        fn branch_remote_tracking_ref_name<'py>(
            &self,
            py: Python<'py>,
            name: &Bound<'_, PyAny>,
            direction: Direction,
        ) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let name = full_name(name)?;
            let result = self.handle.run(py, move |repo| {
                repo.branch_remote_tracking_ref_name(name.as_ref(), direction.inner)
                    .transpose()
                    .map_err(to_py)
            })?;
            Ok(result.map(|value| PyBytes::new(py, value.as_bstr())))
        }
        fn branch_remote_name<'py>(
            &self,
            py: Python<'py>,
            short_branch_name: &Bound<'_, PyAny>,
            direction: Direction,
        ) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let name = bytes(short_branch_name)?;
            let result = self.handle.run(py, move |repo| {
                Ok(repo
                    .branch_remote_name(name.as_bstr(), direction.inner)
                    .map(|value| value.as_bstr().to_vec()))
            })?;
            Ok(result.map(|value| PyBytes::new(py, &value)))
        }
        fn branch_remote(
            &self,
            py: Python<'_>,
            short_branch_name: &Bound<'_, PyAny>,
            direction: Direction,
        ) -> PyResult<Option<Remote>> {
            let name = bytes(short_branch_name)?;
            make_remote(py, self.handle.clone(), move |repo| {
                repo.branch_remote(name.as_bstr(), direction.inner)
                    .transpose()
                    .map_err(to_py)
            })
        }
        fn upstream_branch_and_remote_for_tracking_branch<'py>(
            &self,
            py: Python<'py>,
            tracking_branch: &Bound<'_, PyAny>,
        ) -> PyResult<Option<(Bound<'py, PyBytes>, Remote)>> {
            let name = full_name(tracking_branch)?;
            let upstream = Arc::new(Mutex::new(None));
            let output = upstream.clone();
            let remote = make_remote(py, self.handle.clone(), move |repo| {
                repo.upstream_branch_and_remote_for_tracking_branch(name.as_ref())
                    .map(|value| {
                        value.map(|(name, remote)| {
                            *output.lock().unwrap_or_else(|error| error.into_inner()) = Some(name);
                            remote
                        })
                    })
                    .map_err(to_py)
            })?;
            remote
                .map(|remote| {
                    let name = upstream
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .take()
                        .ok_or_else(|| PyRuntimeError::new_err("native upstream name is missing"))?;
                    Ok((PyBytes::new(py, name.as_bstr()), remote))
                })
                .transpose()
        }
    }

    #[pymethods]
    impl Reference {
        fn remote_name<'py>(&self, py: Python<'py>, direction: Direction) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let reference = self.inner.clone();
            let result = self.handle.run(py, move |repo| {
                Ok(reference
                    .attach(repo)
                    .remote_name(direction.inner)
                    .map(|value| value.as_bstr().to_vec()))
            })?;
            Ok(result.map(|value| PyBytes::new(py, &value)))
        }
        fn remote(&self, py: Python<'_>, direction: Direction) -> PyResult<Option<Remote>> {
            let reference = self.inner.clone();
            make_remote(py, self.handle.clone(), move |repo| {
                reference
                    .attach(repo)
                    .remote(direction.inner)
                    .transpose()
                    .map_err(to_py)
            })
        }
        fn remote_ref_name<'py>(&self, py: Python<'py>, direction: Direction) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let reference = self.inner.clone();
            let result = self.handle.run(py, move |repo| {
                reference
                    .attach(repo)
                    .remote_ref_name(direction.inner)
                    .transpose()
                    .map_err(to_py)
            })?;
            Ok(result.map(|value| PyBytes::new(py, value.as_bstr())))
        }
        fn remote_tracking_ref_name<'py>(
            &self,
            py: Python<'py>,
            direction: Direction,
        ) -> PyResult<Option<Bound<'py, PyBytes>>> {
            let reference = self.inner.clone();
            let result = self.handle.run(py, move |repo| {
                reference
                    .attach(repo)
                    .remote_tracking_ref_name(direction.inner)
                    .transpose()
                    .map_err(to_py)
            })?;
            Ok(result.map(|value| PyBytes::new(py, value.as_bstr())))
        }
    }

    #[pymethods]
    impl Head {
        fn into_remote(&self, py: Python<'_>, direction: Direction) -> PyResult<Option<Remote>> {
            let head = self.inner.clone();
            make_remote(py, self.handle.clone(), move |repo| {
                head.attach(repo)
                    .into_remote(direction.inner)
                    .transpose()
                    .map_err(to_py)
            })
        }
    }
}
