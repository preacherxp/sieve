package sieve.agent;

import java.lang.classfile.ClassFile;
import java.lang.classfile.ClassHierarchyResolver;
import java.lang.classfile.ClassModel;
import java.lang.classfile.CodeBuilder;
import java.lang.classfile.CodeElement;
import java.lang.classfile.CodeTransform;
import java.lang.classfile.MethodModel;
import java.lang.classfile.MethodTransform;
import java.lang.constant.ClassDesc;
import java.lang.constant.ConstantDescs;
import java.lang.constant.MethodTypeDesc;
import java.lang.instrument.ClassFileTransformer;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.CodeSource;
import java.security.ProtectionDomain;
import java.util.List;
import java.util.Set;

/**
 * Adds a method-entry probe to every method of the project's own classes, and a file probe to
 * the JDK constructors and methods that open files.
 */
final class Transformer implements ClassFileTransformer {
    private static final ClassDesc PROBE = ClassDesc.of("sieve.probe.Probe");
    private static final MethodTypeDesc HIT = MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_int);
    private static final MethodTypeDesc FILE = MethodTypeDesc.of(ConstantDescs.CD_void, ConstantDescs.CD_Object);
    private static final Set<String> LISTINGS = Set.of("list", "listFiles", "newDirectoryStream");
    /** File-system provider methods whose first parameter is the path they open, list, or probe. */
    private static final Set<String> FILE_METHODS = Set.of("newByteChannel", "newFileChannel", "newAsynchronousFileChannel",
            "newDirectoryStream", "copy", "checkAccess", "readAttributes", "readAttributesIfExists", "exists",
            "isDirectory", "isRegularFile");
    /** {@code java.io.File} methods that look at the file they are called on. */
    private static final Set<String> SELF_METHODS = Set.of("exists", "isFile", "isDirectory", "length", "list", "listFiles");

    private final Set<Path> outputs = new java.util.HashSet<>();
    private final Set<String> jdk;
    private final java.util.Map<String, java.util.Optional<Path>> locations = new java.util.concurrent.ConcurrentHashMap<>();

    Transformer(List<Path> outputs, Set<String> jdk) {
        // Real paths, so that a symlinked or differently spelled checkout still matches.
        for (Path output : outputs) {
            this.outputs.add(real(output));
        }
        this.jdk = Set.copyOf(jdk);
    }

    private static Path real(Path path) {
        try {
            return path.toRealPath();
        } catch (java.io.IOException error) {
            return path.toAbsolutePath().normalize();
        }
    }

    @Override
    public byte[] transform(ClassLoader loader, String name, Class<?> redefined, ProtectionDomain domain, byte[] bytes) {
        if (name == null) {
            return null;
        }
        try {
            if (jdk.contains(name)) {
                return fileProbes(bytes);
            }
            Path dir = output(domain);
            // Proxies defined into an output directory's domain have no class file there.
            if (dir == null || !Files.isRegularFile(dir.resolve(name + ".class"))) {
                return null;
            }
            byte[] probed = methodProbes(loader, name, bytes);
            State.instrumented();
            return probed;
        } catch (Throwable error) {
            State.error(name + ": " + error);
            return null;
        }
    }

    private Path output(ProtectionDomain domain) throws Exception {
        CodeSource source = domain == null ? null : domain.getCodeSource();
        URL location = source == null ? null : source.getLocation();
        if (location == null || !"file".equals(location.getProtocol())) {
            return null;
        }
        return locations.computeIfAbsent(location.toString(), key -> {
            try {
                Path path = real(Path.of(location.toURI()));
                return java.util.Optional.ofNullable(outputs.contains(path) ? path : null);
            } catch (java.net.URISyntaxException | RuntimeException error) {
                return java.util.Optional.empty();
            }
        }).orElse(null);
    }

    private static ClassFile classFile(ClassLoader loader) {
        ClassLoader resources = loader == null ? ClassLoader.getSystemClassLoader() : loader;
        return ClassFile.of(ClassFile.ClassHierarchyResolverOption.of(
                ClassHierarchyResolver.defaultResolver().orElse(ClassHierarchyResolver.ofResourceParsing(resources))));
    }

    private static byte[] methodProbes(ClassLoader loader, String name, byte[] bytes) {
        ClassFile file = classFile(loader);
        ClassModel model = file.parse(bytes);
        return file.transformClass(model, (builder, element) -> {
            if (element instanceof MethodModel method && method.code().isPresent()) {
                int id = State.method(name + "#" + method.methodName().stringValue() + method.methodType().stringValue());
                builder.transformMethod(method, MethodTransform.transformingCode(new Prologue(b -> {
                    b.loadConstant(id);
                    b.invokestatic(PROBE, "hit", HIT);
                })));
            } else {
                builder.with(element);
            }
        });
    }

    /**
     * Reports the {@code File}, {@code String}, or {@code Path} that a method opens, lists, or
     * probes: the first parameter, or the {@code File} itself.
     */
    private static byte[] fileProbes(byte[] bytes) {
        ClassFile file = classFile(null);
        ClassModel model = file.parse(bytes);
        return file.transformClass(model, (builder, element) -> {
            int slot = element instanceof MethodModel method && method.code().isPresent() ? slot(model, method) : -1;
            if (slot >= 0) {
                String probe = LISTINGS.contains(((MethodModel) element).methodName().stringValue()) ? "list" : "file";
                builder.transformMethod((MethodModel) element, MethodTransform.transformingCode(new Prologue(b -> {
                    b.aload(slot);
                    b.invokestatic(PROBE, probe, FILE);
                })));
            } else {
                builder.with(element);
            }
        });
    }

    /** The local variable slot holding the file a method uses, or -1. */
    private static int slot(ClassModel model, MethodModel method) {
        if ((method.flags().flagsMask() & ClassFile.ACC_STATIC) != 0) {
            return -1;
        }
        String name = method.methodName().stringValue();
        String type = method.methodType().stringValue();
        if (model.thisClass().asInternalName().equals("java/io/File")) {
            return SELF_METHODS.contains(name) ? 0 : -1;
        }
        if (name.equals("<init>")) {
            return type.startsWith("(Ljava/io/File;") ? 1 : -1;
        }
        return FILE_METHODS.contains(name) && type.startsWith("(Ljava/nio/file/Path;") ? 1 : -1;
    }

    private record Prologue(java.util.function.Consumer<CodeBuilder> start) implements CodeTransform {
        @Override
        public void atStart(CodeBuilder builder) {
            start.accept(builder);
        }

        @Override
        public void accept(CodeBuilder builder, CodeElement element) {
            builder.with(element);
        }
    }
}
