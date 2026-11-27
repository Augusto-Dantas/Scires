using IJulia

project = normpath(@__DIR__)
kernel_path = IJulia.installkernel(
    "SciRes Julia",
    "--startup-file=no",
    "--project=$(project)";
    specname="scires-julia",
    displayname="SciRes Julia",
)
println(kernel_path)
