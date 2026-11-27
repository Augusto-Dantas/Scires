module __SciResDataPlane

using Sockets
using SparseArrays

const PROFILE_KEY = "scires.value.envelope"
const PROFILE_ID = "scires.value"
const PROFILE_VERSION = 1

const ARROW_MOD = Ref{Any}(nothing)
const SERVER_REF = Ref{Any}(nothing)
const TOKEN_REF = Ref{String}("")
const RUNNING_REF = Ref{Bool}(false)

function _ensure_arrow()
    if ARROW_MOD[] === nothing
        try
            ARROW_MOD[] = Base.require(Base.PkgId(Base.UUID("69666777-d1a9-59fb-9406-91d4454c9d45"), "Arrow"))
        catch
            return nothing
        end
    end
    return ARROW_MOD[]
end

# --- Minimal self-contained JSON helpers (no external JSON dep required) ---

function _json_escape(s::AbstractString)
    io = IOBuffer()
    write(io, '"')
    for c in s
        if c == '"'
            write(io, "\\\"")
        elseif c == '\\'
            write(io, "\\\\")
        elseif c == '\n'
            write(io, "\\n")
        elseif c == '\r'
            write(io, "\\r")
        elseif c == '\t'
            write(io, "\\t")
        elseif UInt32(c) < 0x20
            write(io, "\\u", lpad(string(UInt32(c), base=16), 4, '0'))
        else
            write(io, c)
        end
    end
    write(io, '"')
    return String(take!(io))
end

function _skip_ws(s::AbstractString, i::Int)
    n = lastindex(s)
    while i <= n
        c = s[i]
        if c == ' ' || c == '\t' || c == '\n' || c == '\r'
            i = nextind(s, i)
        else
            break
        end
    end
    return i
end

function _parse_json_string(s::AbstractString, i::Int)
    s[i] == '"' || error("expected '\"'")
    i = nextind(s, i)
    io = IOBuffer()
    n = lastindex(s)
    while i <= n
        c = s[i]
        if c == '"'
            return String(take!(io)), nextind(s, i)
        elseif c == '\\'
            i = nextind(s, i)
            i <= n || error("unterminated escape")
            esc = s[i]
            if esc == '"' || esc == '\\' || esc == '/'
                write(io, esc)
            elseif esc == 'n'
                write(io, '\n')
            elseif esc == 'r'
                write(io, '\r')
            elseif esc == 't'
                write(io, '\t')
            elseif esc == 'b'
                write(io, '\b')
            elseif esc == 'f'
                write(io, '\f')
            elseif esc == 'u'
                i = nextind(s, i)
                hex_end = i
                for _ in 1:3
                    hex_end = nextind(s, hex_end)
                end
                code = parse(UInt32, s[i:hex_end], base=16)
                write(io, Char(code))
                i = hex_end
            else
                error("unsupported JSON escape: \\$esc")
            end
            i = nextind(s, i)
        else
            write(io, c)
            i = nextind(s, i)
        end
    end
    error("unterminated JSON string")
end

function _parse_json_value(s::AbstractString, i::Int)
    i = _skip_ws(s, i)
    c = s[i]
    if c == '"'
        return _parse_json_string(s, i)
    elseif c == '{'
        i = nextind(s, i)
        obj = Dict{String, Any}()
        i = _skip_ws(s, i)
        if s[i] == '}'
            return obj, nextind(s, i)
        end
        while true
            i = _skip_ws(s, i)
            k, i = _parse_json_string(s, i)
            i = _skip_ws(s, i)
            s[i] == ':' || error("expected ':'")
            i = nextind(s, i)
            v, i = _parse_json_value(s, i)
            obj[k] = v
            i = _skip_ws(s, i)
            if s[i] == '}'
                return obj, nextind(s, i)
            elseif s[i] == ','
                i = nextind(s, i)
            else
                error("expected ',' or '}'")
            end
        end
    elseif c == '['
        i = nextind(s, i)
        arr = Any[]
        i = _skip_ws(s, i)
        if s[i] == ']'
            return arr, nextind(s, i)
        end
        while true
            v, i = _parse_json_value(s, i)
            push!(arr, v)
            i = _skip_ws(s, i)
            if s[i] == ']'
                return arr, nextind(s, i)
            elseif s[i] == ','
                i = nextind(s, i)
            else
                error("expected ',' or ']'")
            end
        end
    elseif startswith(SubString(s, i), "true")
        return true, i + 4
    elseif startswith(SubString(s, i), "false")
        return false, i + 5
    elseif startswith(SubString(s, i), "null")
        return nothing, i + 4
    else
        j = i
        n = lastindex(s)
        while j <= n && (s[j] in "0123456789-+eE.")
            j = nextind(s, j)
        end
        tok = s[i:prevind(s, j)]
        val = occursin('.', tok) || occursin('e', tok) || occursin('E', tok) ?
            parse(Float64, tok) : parse(Int64, tok)
        return val, j
    end
end

function _parse_json(s::AbstractString)
    val, _ = _parse_json_value(s, firstindex(s))
    return val
end

# --- SciRes Arrow v1 profile conversion ---

function _dtype_name(::Type{T}) where {T}
    T === Bool && return "bool"
    T === Int8 && return "int8"
    T === Int16 && return "int16"
    T === Int32 && return "int32"
    T === Int64 && return "int64"
    T === UInt8 && return "u_int8"
    T === UInt16 && return "u_int16"
    T === UInt32 && return "u_int32"
    T === UInt64 && return "u_int64"
    T === Float32 && return "float32"
    T === Float64 && return "float64"
    T === ComplexF32 && return "complex64"
    T === ComplexF64 && return "complex128"
    return nothing
end

function _is_supported_scalar(val)
    val === nothing || val === missing ||
    val isa Bool || val isa Int8 || val isa Int16 || val isa Int32 || val isa Int64 ||
    val isa UInt8 || val isa UInt16 || val isa UInt32 || val isa UInt64 ||
    val isa Float32 || val isa Float64 || val isa String ||
    val isa ComplexF32 || val isa ComplexF64
end

function _is_supported_table_eltype(::Type{T}) where {T}
    U = Base.nonmissingtype(T)
    U === Missing && return true
    U === Bool || U === Int8 || U === Int16 || U === Int32 || U === Int64 ||
    U === UInt8 || U === UInt16 || U === UInt32 || U === UInt64 ||
    U === Float16 || U === Float32 || U === Float64 ||
    U === String || U === Vector{UInt8}
end

function _is_namedtuple_table(val)
    val isa NamedTuple || return false
    isempty(val) && return false
    len = nothing
    for col in values(val)
        col isa AbstractVector || return false
        if len === nothing
            len = length(col)
        elseif length(col) != len
            error("ragged table columns cannot be transferred losslessly")
        end
        _is_supported_table_eltype(eltype(col)) ||
            error("unsupported table column element type $(eltype(col))")
    end
    return true
end

function _export_arrow(val)
    Arrow = _ensure_arrow()
    Arrow === nothing && error("Arrow.jl is not available in this Julia environment")

    if _is_supported_scalar(val)
        envelope = "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"scalar\"}}"
        col = if val === nothing || val === missing
            Missing[missing]
        elseif val isa ComplexF32
            [(real = Float32(real(val)), imaginary = Float32(imag(val)))]
        elseif val isa ComplexF64
            [(real = Float64(real(val)), imaginary = Float64(imag(val)))]
        else
            [val]
        end
        io = IOBuffer()
        Arrow.write(io, (value = col,); metadata = Dict(PROFILE_KEY => envelope))
        return take!(io)
    elseif val isa SparseMatrixCSC
        T = eltype(val)
        dtype = _dtype_name(T)
        if dtype === nothing || dtype == "bool"
            error("unsupported CSC sparse matrix element type $T")
        end
        nrows, ncols = size(val)
        envelope = string(
            "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"sparse_csc\",\"dtype\":",
            _json_escape(dtype),
            ",\"shape\":[", nrows, ",", ncols, "],\"index_base\":0}}"
        )
        nzvals = if T === ComplexF32
            [(real = Float32(real(z)), imaginary = Float32(imag(z))) for z in val.nzval]
        elseif T === ComplexF64
            [(real = Float64(real(z)), imaginary = Float64(imag(z))) for z in val.nzval]
        else
            copy(val.nzval)
        end
        table = (
            indptr = [Int64.(val.colptr .- 1)],
            indices = [Int64.(val.rowval .- 1)],
            values = [nzvals],
        )
        io = IOBuffer()
        Arrow.write(io, table; metadata = Dict(PROFILE_KEY => envelope), largelists = true)
        return take!(io)
    elseif val isa Array
        ndims(val) >= 1 || error("0-dimensional arrays are not supported as tensors; use a scalar")
        T = eltype(val)
        dtype = _dtype_name(T)
        dtype === nothing &&
            error("unsupported dense tensor element type $T; dense tensors must be fixed-width numeric/Bool without missing values")
        dims_str = join(size(val), ",")
        envelope = string(
            "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"tensor\",\"dtype\":",
            _json_escape(dtype),
            ",\"shape\":[", dims_str, "],\"layout\":\"column_major\",\"axes\":[]}}"
        )
        flat = vec(val)
        col = if T === ComplexF32
            [(real = Float32(real(z)), imaginary = Float32(imag(z))) for z in flat]
        elseif T === ComplexF64
            [(real = Float64(real(z)), imaginary = Float64(imag(z))) for z in flat]
        else
            flat
        end
        io = IOBuffer()
        Arrow.write(io, (values = col,); metadata = Dict(PROFILE_KEY => envelope))
        return take!(io)
    elseif _is_namedtuple_table(val)
        envelope = "{\"profile\":\"scires.value\",\"version\":1,\"value\":{\"kind\":\"table\"}}"
        io = IOBuffer()
        Arrow.write(io, val; metadata = Dict(PROFILE_KEY => envelope))
        return take!(io)
    else
        error("unsupported Julia value of type $(typeof(val)); convert explicitly before transfer")
    end
end

function _materialize_tensor_values(raw, dtype::AbstractString)
    if dtype == "complex64"
        return ComplexF32[ComplexF32(v.real, v.imaginary) for v in raw]
    elseif dtype == "complex128"
        return ComplexF64[ComplexF64(v.real, v.imaginary) for v in raw]
    else
        vals = collect(raw)
        any(ismissing, vals) && error("dense tensor elements cannot be missing")
        T = Base.nonmissingtype(eltype(vals))
        return T[v for v in vals]
    end
end

function _import_arrow(payload::Vector{UInt8})
    Arrow = _ensure_arrow()
    Arrow === nothing && error("Arrow.jl is not available in this Julia environment")

    table = Arrow.Table(payload)
    meta = Arrow.getmetadata(table)
    (meta !== nothing && haskey(meta, PROFILE_KEY)) ||
        error("Arrow stream is missing SciRes value envelope")
    env = _parse_json(meta[PROFILE_KEY])
    env["profile"] == PROFILE_ID || error("unsupported value profile $(env["profile"])")
    env["version"] == PROFILE_VERSION || error("unsupported value profile version $(env["version"])")

    vmeta = env["value"]
    kind = vmeta["kind"]

    if kind == "scalar"
        v = table.value[1]
        if ismissing(v)
            return missing
        elseif v isa NamedTuple && haskey(v, :real) && haskey(v, :imaginary)
            return Complex(v.real, v.imaginary)
        elseif v isa AbstractString
            return String(v)
        else
            return v
        end
    elseif kind == "table"
        col_names = Tuple(Symbol.(Arrow.Tables.columnnames(table)))
        cols = Tuple(
            begin
                c = collect(Arrow.Tables.getcolumn(table, name))
                c
            end
            for name in col_names
        )
        return NamedTuple{col_names}(cols)
    elseif kind == "tensor"
        vmeta["layout"] == "column_major" || error("unsupported tensor layout $(vmeta["layout"])")
        shape = Int.(vmeta["shape"])
        vals = _materialize_tensor_values(table.values, vmeta["dtype"])
        return reshape(vals, Tuple(shape))
    elseif kind == "sparse_csc"
        vmeta["index_base"] == 0 || error("wire index_base must be 0")
        shape = Int.(vmeta["shape"])
        wire_colptr = Int.(collect(table.indptr[1]))
        wire_rows = Int.(collect(table.indices[1]))
        vals = _materialize_tensor_values(table.values[1], vmeta["dtype"])
        return SparseMatrixCSC(shape[1], shape[2], wire_colptr .+ 1, wire_rows .+ 1, vals)
    else
        error("unsupported SciRes value kind $kind")
    end
end

# --- Loopback binary TCP framing ---

function _read_frame(sock)
    hdr_len = ntoh(read(sock, UInt32))
    hdr_len <= 65536 || error("data-plane header too large")
    hdr_bytes = read(sock, hdr_len)
    length(hdr_bytes) == hdr_len || error("truncated data-plane header")
    hdr = _parse_json(String(hdr_bytes))

    payload_len = ntoh(read(sock, UInt64))
    payload = payload_len == 0 ? UInt8[] : read(sock, payload_len)
    length(payload) == payload_len || error("truncated data-plane payload")
    return hdr, payload
end

function _write_frame(sock, hdr_json::String, payload::Vector{UInt8})
    hdr_bytes = Vector{UInt8}(codeunits(hdr_json))
    write(sock, hton(UInt32(length(hdr_bytes))))
    write(sock, hdr_bytes)
    write(sock, hton(UInt64(length(payload))))
    if !isempty(payload)
        write(sock, payload)
    end
    flush(sock)
end

function _handle_client(sock)
    try
        hdr, payload = _read_frame(sock)
        if get(hdr, "token", "") != TOKEN_REF[]
            _write_frame(sock, "{\"ok\":false,\"error\":\"invalid data-plane token\"}", UInt8[])
            return
        end
        op = get(hdr, "op", "")
        if op == "ping"
            _write_frame(sock, "{\"ok\":true}", UInt8[])
        elseif op == "export"
            name = Symbol(hdr["name"])
            if !Base.invokelatest(isdefined, Main, name)
                _write_frame(
                    sock,
                    string("{\"ok\":false,\"error\":", _json_escape("variable `$name` is not defined in Main"), "}"),
                    UInt8[],
                )
                return
            end
            val = Base.invokelatest(getfield, Main, name)
            bytes = Base.invokelatest(_export_arrow, val)
            _write_frame(sock, "{\"ok\":true}", bytes)
        elseif op == "import"
            name = Symbol(hdr["name"])
            val = Base.invokelatest(_import_arrow, payload)
            Core.eval(Main, :($name = $val))
            _write_frame(sock, "{\"ok\":true}", UInt8[])
        elseif op == "shutdown"
            RUNNING_REF[] = false
            _write_frame(sock, "{\"ok\":true}", UInt8[])
            if SERVER_REF[] !== nothing
                try
                    close(SERVER_REF[])
                catch
                end
                SERVER_REF[] = nothing
            end
        else
            _write_frame(
                sock,
                string("{\"ok\":false,\"error\":", _json_escape("unknown op `$op`"), "}"),
                UInt8[],
            )
        end
    catch e
        msg = sprint(showerror, e)
        try
            _write_frame(
                sock,
                string("{\"ok\":false,\"error\":", _json_escape(msg), "}"),
                UInt8[],
            )
        catch
        end
    finally
        try
            close(sock)
        catch
        end
    end
end

function start(token::String)
    if _ensure_arrow() === nothing
        return "UNAVAILABLE"
    end
    if SERVER_REF[] !== nothing
        try
            close(SERVER_REF[])
        catch
        end
        SERVER_REF[] = nothing
    end
    TOKEN_REF[] = token
    RUNNING_REF[] = true
    port, server = Sockets.listenany(Sockets.localhost, 0)
    SERVER_REF[] = server
    @async begin
        while RUNNING_REF[]
            sock = try
                accept(server)
            catch
                break
            end
            @async _handle_client(sock)
        end
    end
    return "PORT:$(Int(port))"
end

end
