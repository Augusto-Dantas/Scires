using Arrow
using SparseArrays

const PROFILE_KEY = "scires.value.envelope"
const TENSOR_ENVELOPE =
    "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"tensor\",\"dtype\":\"int32\",\"shape\":[2,3],\"layout\":\"column_major\",\"axes\":[{\"name\":\"row\",\"unit\":\"m\"},{\"name\":\"column\",\"unit\":null}]}}"
const SPARSE_ENVELOPE =
    "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"sparse_csc\",\"dtype\":\"float64\",\"shape\":[3,2],\"index_base\":0}}"
const TABLE_ENVELOPE =
    "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"table\"}}"
const SCALAR_ENVELOPE =
    "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"scalar\"}}"

function read_metadata(path)
    table = Arrow.Table(path)
    metadata = Arrow.getmetadata(table)
    haskey(metadata, PROFILE_KEY) || error("SciRes value envelope is missing")
    return table, metadata[PROFILE_KEY]
end

function read_tensor(path)
    table, envelope = read_metadata(path)
    occursin("\"kind\":\"tensor\"", envelope) || error("expected tensor envelope")
    occursin("\"dtype\":\"int32\"", envelope) || error("expected Int32 tensor")
    occursin("\"shape\":[2,3]", envelope) || error("unexpected tensor shape")
    occursin("\"layout\":\"column_major\"", envelope) || error("unexpected layout")
    occursin("\"unit\":\"m\"", envelope) || error("axis unit was not preserved")
    eltype(table.values) == Int32 || error("tensor values did not remain Int32")
    collect(table.values) == Int32[1, 2, 3, 4, 5, 6] ||
        error("tensor values did not round-trip")
end

function read_sparse(path)
    table, envelope = read_metadata(path)
    occursin("\"kind\":\"sparse_csc\"", envelope) || error("expected CSC envelope")
    occursin("\"shape\":[3,2]", envelope) || error("unexpected sparse shape")
    occursin("\"index_base\":0", envelope) || error("wire index base must be zero")

    wire_colptr = Int.(collect(table.indptr[1]))
    wire_rows = Int.(collect(table.indices[1]))
    values = collect(table.values[1])
    matrix = SparseMatrixCSC(
        3,
        2,
        wire_colptr .+ 1,
        wire_rows .+ 1,
        values,
    )
    matrix.colptr == [1, 3, 4] || error("CSC offsets were not converted to Julia indexing")
    matrix.rowval == [1, 3, 2] || error("CSC indices were not converted to Julia indexing")
    matrix.nzval == [10.0, 20.0, 30.0] || error("CSC values did not round-trip")
end

function read_table(path)
    table, envelope = read_metadata(path)
    occursin("\"kind\":\"table\"", envelope) || error("expected table envelope")
    collect(table.id) == Int64[1, 2] || error("table integer column did not round-trip")
    isnan(table.value[1]) || error("valid NaN was not preserved")
    ismissing(table.value[2]) || error("null was not preserved as missing")
end

function read_scalar(path)
    table, envelope = read_metadata(path)
    occursin("\"kind\":\"scalar\"", envelope) || error("expected scalar envelope")
    table.value[1] == Int32(42) || error("scalar did not round-trip")
end

function write_tensor(path)
    table = (values=Int32[1, 2, 3, 4, 5, 6],)
    open(path, "w") do io
        Arrow.write(io, table; metadata=Dict(PROFILE_KEY => TENSOR_ENVELOPE))
    end
end

function write_sparse(path)
    matrix = SparseMatrixCSC(
        3,
        2,
        [1, 3, 4],
        [1, 3, 2],
        [10.0, 20.0, 30.0],
    )
    table = (
        indptr=[Int64.(matrix.colptr .- 1)],
        indices=[Int64.(matrix.rowval .- 1)],
        values=[copy(matrix.nzval)],
    )
    open(path, "w") do io
        Arrow.write(
            io,
            table;
            metadata=Dict(PROFILE_KEY => SPARSE_ENVELOPE),
            largelists=true,
        )
    end
end

function write_table(path)
    table = (
        id=Int64[1, 2],
        value=Union{Missing,Float64}[NaN, missing],
    )
    open(path, "w") do io
        Arrow.write(io, table; metadata=Dict(PROFILE_KEY => TABLE_ENVELOPE))
    end
end

function write_scalar(path)
    table = (value=Int32[42],)
    open(path, "w") do io
        Arrow.write(io, table; metadata=Dict(PROFILE_KEY => SCALAR_ENVELOPE))
    end
end

mode, path = ARGS
if mode == "read_tensor"
    read_tensor(path)
elseif mode == "read_sparse"
    read_sparse(path)
elseif mode == "read_table"
    read_table(path)
elseif mode == "read_scalar"
    read_scalar(path)
elseif mode == "write_tensor"
    write_tensor(path)
elseif mode == "write_sparse"
    write_sparse(path)
elseif mode == "write_table"
    write_table(path)
elseif mode == "write_scalar"
    write_scalar(path)
else
    error("unsupported mode: $mode")
end
