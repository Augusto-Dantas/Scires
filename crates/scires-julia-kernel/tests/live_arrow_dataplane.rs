//! Live Rust <-> Julia Arrow IPC v1 data-plane integration tests.
//!
//! Run with:
//! `SCIRES_JULIA_PROJECT=/tmp/scires-julia-test-env cargo test -p scires-julia-kernel -- --ignored --nocapture`

use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use scires_interchange::{
    decode, encode, AxisMetadata, Complex64, CscMatrix, DenseTensor, InterchangeValue, ScalarValue,
    TensorData,
};
use scires_julia_kernel::{JuliaKernel, JuliaKernelConfig};
use scires_kernel::{ExecuteRequest, Kernel, KernelError, KernelStatus, Output, ValueProfile};
use std::path::PathBuf;
use std::sync::Arc;

fn live_config() -> JuliaKernelConfig {
    let project = std::env::var_os("SCIRES_JULIA_PROJECT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(|p| p.parent())
                .expect("workspace root")
                .join("tests")
                .join("julia")
        });
    JuliaKernelConfig::discover(project).expect("Julia executable should be discoverable")
}

fn eval_text(kernel: &mut JuliaKernel, code: &str) -> String {
    let res = kernel
        .execute(&ExecuteRequest::new(code))
        .unwrap_or_else(|err| panic!("Julia code `{code}` failed: {err}"));
    res.outputs
        .iter()
        .find_map(|output| match output {
            Output::Display { mime, data } if mime == "text/plain" => Some(data.trim().to_owned()),
            Output::Stdout(s) => Some(s.trim().to_owned()),
            _ => None,
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "requires an instantiated Julia/IJulia+Arrow project (set SCIRES_JULIA_PROJECT)"]
fn live_arrow_dataplane_round_trips_and_rejects_unsupported_values() {
    let mut kernel = JuliaKernel::new(live_config());
    kernel.start().expect("Julia kernel should start");
    assert_eq!(kernel.status(), KernelStatus::Idle);

    // 1. Handshake advertises scires.value v1 when Arrow.jl is available.
    let profile = ValueProfile::scires_v1();
    assert!(
        kernel.handshake().value_profiles.contains(&profile),
        "handshake must advertise scires.value v1: {:?}",
        kernel.handshake()
    );

    // 2. Scalars: Int32, NaN, Null, UTF-8, Complex128.
    let int_bytes = encode(&InterchangeValue::Scalar(ScalarValue::Int32(Some(42)))).unwrap();
    kernel.import_value("s_int", &profile, &int_bytes).unwrap();
    assert_eq!(eval_text(&mut kernel, "s_int === Int32(42)"), "true");
    let got_int = decode(&kernel.export_value("s_int", &profile).unwrap()).unwrap();
    assert!(matches!(
        got_int,
        InterchangeValue::Scalar(ScalarValue::Int32(Some(42)))
    ));

    let nan_bytes =
        encode(&InterchangeValue::Scalar(ScalarValue::Float64(Some(f64::NAN)))).unwrap();
    kernel.import_value("s_nan", &profile, &nan_bytes).unwrap();
    assert_eq!(eval_text(&mut kernel, "isnan(s_nan)"), "true");
    let got_nan = decode(&kernel.export_value("s_nan", &profile).unwrap()).unwrap();
    assert!(matches!(
        got_nan,
        InterchangeValue::Scalar(ScalarValue::Float64(Some(v))) if v.is_nan()
    ));

    let null_bytes = encode(&InterchangeValue::Scalar(ScalarValue::Null)).unwrap();
    kernel
        .import_value("s_null", &profile, &null_bytes)
        .unwrap();
    assert_eq!(eval_text(&mut kernel, "ismissing(s_null)"), "true");
    let got_null = decode(&kernel.export_value("s_null", &profile).unwrap()).unwrap();
    assert!(matches!(
        got_null,
        InterchangeValue::Scalar(ScalarValue::Null)
    ));

    let str_bytes =
        encode(&InterchangeValue::Scalar(ScalarValue::Utf8(Some("scires-λ".into())))).unwrap();
    kernel.import_value("s_str", &profile, &str_bytes).unwrap();
    assert_eq!(eval_text(&mut kernel, "s_str == \"scires-λ\""), "true");
    let got_str = decode(&kernel.export_value("s_str", &profile).unwrap()).unwrap();
    assert!(matches!(
        got_str,
        InterchangeValue::Scalar(ScalarValue::Utf8(Some(ref s))) if s == "scires-λ"
    ));

    let c128 = Complex64 {
        real: 1.5,
        imaginary: -2.25,
    };
    let c_bytes = encode(&InterchangeValue::Scalar(ScalarValue::Complex128(Some(c128)))).unwrap();
    kernel
        .import_value("s_complex", &profile, &c_bytes)
        .unwrap();
    assert_eq!(
        eval_text(&mut kernel, "s_complex === ComplexF64(1.5, -2.25)"),
        "true"
    );
    let got_c = decode(&kernel.export_value("s_complex", &profile).unwrap()).unwrap();
    assert!(matches!(
        got_c,
        InterchangeValue::Scalar(ScalarValue::Complex128(Some(v))) if v == c128
    ));

    // 3. Table preserving valid NaN separately from missing/null.
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("value", DataType::Float64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Float64Array::from(vec![Some(f64::NAN), None, Some(3.5)])),
        ],
    )
    .unwrap();
    let table_bytes = encode(&InterchangeValue::Table(batch)).unwrap();
    kernel
        .import_value("tbl", &profile, &table_bytes)
        .unwrap();
    assert_eq!(
        eval_text(
            &mut kernel,
            "tbl.id == Int64[1, 2, 3] && isnan(tbl.value[1]) && ismissing(tbl.value[2]) && tbl.value[3] == 3.5"
        ),
        "true"
    );
    let InterchangeValue::Table(rt_batch) =
        decode(&kernel.export_value("tbl", &profile).unwrap()).unwrap()
    else {
        panic!("expected Table");
    };
    let vals = rt_batch
        .column(1)
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    assert!(vals.is_valid(0) && vals.value(0).is_nan());
    assert!(vals.is_null(1));
    assert_eq!(vals.value(2), 3.5);

    // 4. Dense column-major Int32 and Complex128 tensors.
    let int_tensor = DenseTensor {
        shape: vec![2, 3],
        axes: vec![
            AxisMetadata {
                name: Some("row".into()),
                unit: Some("m".into()),
            },
            AxisMetadata {
                name: Some("col".into()),
                unit: None,
            },
        ],
        data: TensorData::Int32(vec![1, 2, 3, 4, 5, 6]),
    };
    let t_bytes = encode(&InterchangeValue::Tensor(int_tensor)).unwrap();
    kernel.import_value("t_i32", &profile, &t_bytes).unwrap();
    // Column-major [1,2,3,4,5,6] with shape (2,3) => [1 3 5; 2 4 6]
    assert_eq!(
        eval_text(
            &mut kernel,
            "size(t_i32) == (2, 3) && eltype(t_i32) == Int32 && t_i32[1, 1] == 1 && t_i32[2, 1] == 2 && t_i32[1, 2] == 3 && t_i32[2, 3] == 6"
        ),
        "true"
    );
    let InterchangeValue::Tensor(rt_t) =
        decode(&kernel.export_value("t_i32", &profile).unwrap()).unwrap()
    else {
        panic!("expected Tensor");
    };
    assert_eq!(rt_t.shape, vec![2, 3]);
    assert_eq!(rt_t.data, TensorData::Int32(vec![1, 2, 3, 4, 5, 6]));

    let c_tensor = DenseTensor {
        shape: vec![2, 1],
        axes: vec![],
        data: TensorData::Complex128(vec![
            Complex64 {
                real: 1.0,
                imaginary: 2.0,
            },
            Complex64 {
                real: -3.0,
                imaginary: 4.5,
            },
        ]),
    };
    let ct_bytes = encode(&InterchangeValue::Tensor(c_tensor.clone())).unwrap();
    kernel.import_value("t_c128", &profile, &ct_bytes).unwrap();
    assert_eq!(
        eval_text(
            &mut kernel,
            "size(t_c128) == (2, 1) && t_c128[1, 1] === ComplexF64(1.0, 2.0) && t_c128[2, 1] === ComplexF64(-3.0, 4.5)"
        ),
        "true"
    );
    let InterchangeValue::Tensor(rt_ct) =
        decode(&kernel.export_value("t_c128", &profile).unwrap()).unwrap()
    else {
        panic!("expected Complex128 Tensor");
    };
    assert_eq!(rt_ct, c_tensor);

    // 5. Float64 CSC sparse matrix: 0-based wire <-> 1-based Julia SparseMatrixCSC.
    let sparse = CscMatrix {
        shape: [3, 2],
        indptr: vec![0, 2, 3],
        indices: vec![0, 2, 1],
        values: TensorData::Float64(vec![10.0, 20.0, 30.0]),
    };
    let sp_bytes = encode(&InterchangeValue::SparseCsc(sparse.clone())).unwrap();
    kernel.import_value("sp_mat", &profile, &sp_bytes).unwrap();
    assert_eq!(
        eval_text(
            &mut kernel,
            "using SparseArrays; sp_mat isa SparseMatrixCSC && size(sp_mat) == (3, 2) && sp_mat.colptr == [1, 3, 4] && sp_mat.rowval == [1, 3, 2] && sp_mat.nzval == [10.0, 20.0, 30.0]"
        ),
        "true"
    );
    let InterchangeValue::SparseCsc(rt_sp) =
        decode(&kernel.export_value("sp_mat", &profile).unwrap()).unwrap()
    else {
        panic!("expected SparseCsc");
    };
    assert_eq!(rt_sp, sparse);

    // 6. Explicit rejection of unsupported Julia value leaves kernel healthy.
    kernel
        .execute(&ExecuteRequest::new(
            "struct SciResOpaque; x::Int; end; bad_obj = SciResOpaque(1); bad_fn = x -> x + 1",
        ))
        .unwrap();
    let obj_err = kernel.export_value("bad_obj", &profile).unwrap_err();
    assert!(
        matches!(obj_err, KernelError::Transfer(_)),
        "expected Transfer error, got {obj_err:?}"
    );
    let fn_err = kernel.export_value("bad_fn", &profile).unwrap_err();
    assert!(
        matches!(fn_err, KernelError::Transfer(_)),
        "expected Transfer error, got {fn_err:?}"
    );
    assert_eq!(eval_text(&mut kernel, "100 + 23"), "123");

    // 7. Restart re-initializes the data plane cleanly.
    kernel.restart().expect("restart should succeed");
    assert!(kernel.handshake().value_profiles.contains(&profile));
    assert!(matches!(
        kernel.export_value("s_int", &profile),
        Err(KernelError::Transfer(_))
    ));
    kernel.import_value("s_int", &profile, &int_bytes).unwrap();
    assert_eq!(eval_text(&mut kernel, "s_int === Int32(42)"), "true");

    kernel.shutdown().expect("shutdown should succeed");
    assert_eq!(kernel.status(), KernelStatus::Stopped);
}
