The `--dev`, `--group`, or `--optional` flags can be used to remove a dependency from a specific
table.

The `tool.uv.sources` table extends the standard dependency tables with alternative dependency
sources, which are used during development.

To add a Git dependency source, prefix a Git-compatible URL with `git+`.

Optional dependencies can have entries in `tool.uv.sources` the same as normal dependencies.

Development dependencies can be divided into multiple groups, using the `--group` flag.

Once groups are defined, the `--all-groups`, `--no-default-groups`, `--group`, `--only-group`, and
`--no-group` options can be used to include or exclude their dependencies.

If dependencies declared in one group are not compatible with those in another group, uv will fail
to resolve the requirements of the project with an error.

An included group's dependencies cannot conflict with the other dependencies declared in a group.

By default, uv includes the `dev` dependency group in the environment (e.g., during `uv run` or
`uv sync`). The default groups to include can be changed using the `tool.uv.default-groups` setting.

By default, dependency groups must be compatible with your project's `requires-python` range.

Dependencies declared in this section will be combined with the contents in the
`dependency-groups.dev`. Eventually, the `dev-dependencies` field will be deprecated and removed.

Then the `child` workspace member would not be installed, but the transitive dependency `anyio`
would be.

This guide shows how to run Python scripts on the cloud using uv for dependency management and
Coiled for cloud deployment.

Using inline script metadata makes the script fully self-contained: it includes the information that
is needed to run it. This makes it easier to run on other machines, like a machine in the cloud.

Coiled makes it simple to run code on cloud hardware.

You'll be prompted to create a Coiled account if you don't already have one — it's free to start
using Coiled.

The same process that previously ran locally is now running on a remote cloud VM on AWS.

This guide will discuss converting from a `pip` and `pip-tools` workflow centered on `requirements`
files to uv's project workflow using a `pyproject.toml` and `uv.lock` file.

We'll start with an overview of developing with `pip`, then discuss migrating to uv.

This installs the package into the environment that `pip` is installed in. This may be a virtual
environment, or, the global environment of your system's Python installation.

Notice above that `fastapi` is not "locked" to a specific version — each person working on the
project may have a different version of `fastapi` installed. `pip-tools` was created to improve this
experience.

Notice there's a version constraint on `pydantic` — this means only `pydantic` versions later than
`2.0.0` can be used. In contrast, `fastapi` does not have a version constraint — any version can be
used.

After compiling dependencies into a locked set of versions, these files are committed to version
control and distributed with the project.

When compiling dependencies with `pip` or `pip-tools`, the result is only usable on the same
platform as it is generated on. This poses a problem for projects which need to be usable on
multiple platforms, such as Windows and macOS.

When using `pip` and `pip-tools`, a project needs to declare a requirements lock file for each
supported platform.

This resolution mode is also used when using a `pyproject.toml` and `uv.lock`.

The `pyproject.toml` is a standardized file for Python project metadata. It replaces
`requirements.in` files, allowing you to represent arbitrary groups of project dependencies. It also
provides a centralized location for metadata about your project, such as the build system or tool
settings.

However, there is some nuance to this transition. Notice we used the `requirements.in` file, which
does not pin to exact versions of packages so uv will solve for new versions of these packages. You
may want to continue using your previously locked versions from your `requirements.txt` so, when
switching over to uv, none of your dependency versions change.

Your existing versions will be retained when producing a `uv.lock` file.

If your platform-specific dependencies have been compiled into separate files, you can still
transition to a universal lockfile. However, you cannot just use `-c` to specify constraints from
your existing platform-specific `requirements.txt` files because they do not include markers
describing the environment and will consequently conflict.

When using `-o`, uv will constrain the versions to match the existing output file, if it can.

Markers can be added for other platforms by changing the `--python-platform` and `-o` values for
each requirements file you need to import, e.g., to `linux` and `macos`.

Prior to every `uv run` invocation, uv will verify that the lockfile is up-to-date with the
`pyproject.toml`, and that the environment is up-to-date with the lockfile, keeping your project
in-sync without the need for manual intervention. `uv run` guarantees that your command is run in a
consistent, locked environment.

The project environment can also be explicitly created with `uv sync`, e.g., for use with editors.

A Python script is a file intended for standalone execution, e.g., with `python <script>.py`. Using
uv to execute scripts ensures that script dependencies are managed without manually managing
environments.

Multiple dependencies can be requested by repeating the `--with` option.

The inline metadata format allows the dependencies for a script to be declared in the script itself.

Running `uv lock --script` will create a `.lock` file adjacent to the script (e.g.,
`example.py.lock`).

Once locked, subsequent operations like `uv run --script`, `uv add --script`, `uv export --script`,
and `uv tree --script` will reuse the locked dependencies, updating the lockfile if necessary.

If no such lockfile is present, commands like `uv export --script` will still function as expected,
but will not create a lockfile.

In addition to locking dependencies, uv supports an `exclude-newer` field in the `tool.uv` section
of inline script metadata to limit uv to only considering distributions released before a specific
date. This is useful for improving the reproducibility of your script when run at a later point in
time.

An extremely fast Python package and project manager, written in Rust.

Migrate to uv without changing your existing workflows — and experience a 10-100x speedup — with the
`uv pip` interface.

Informally, the intent is such that existing `pip` and `pip-tools` users can switch to uv without
making meaningful changes to their packaging workflows; and, in most cases, swapping out
`pip install` for `uv pip install` should "just work".

This document outlines the known differences between uv and `pip`, along with rationale,
workarounds, and a statement of intent for compatibility in the future.

By default (`if-necessary`), uv prefers stable versions over pre-releases, falling back to
pre-releases only if every stable candidate that satisfies the active constraints is rejected during
resolution.

For example, suppose only `c==1.0` and `c==2.0a1` are available. Together, `c>=1` and
`a -> c>=0.5a1` allow both versions, but uv selects `c==1.0` regardless of which requirement is
discovered first. If another active requirement rejects `c==1.0`, uv falls back to `c==2.0a1`.

Use `--prerelease allow` to consider pre-releases for every package without preferring stable
candidates first, or `--prerelease disallow` to exclude them entirely.

The `explicit` mode considers pre-releases only for first-party requirements that contain a
pre-release identifier (preferring stable versions and falling back to pre-releases only if
necessary), while disallowing pre-releases for all other packages.

Prior to pip 26.0, this behavior was not consistent.

In both uv and `pip`, users can specify multiple package indexes from which to search for the
available versions of a given package. However, uv and `pip` differ in how they handle packages that
exist on multiple indexes.

When uv searches for a package across multiple indexes, it will iterate over the indexes in order
(preferring the `--extra-index-url` over the default index), and stop searching as soon as it finds
a match. This means that if a package exists on multiple indexes, uv will limit its candidate
versions to those present in the first index that contains the package.

While `unsafe-best-match` is the closest to `pip`'s behavior, it exposes users to the risk of
"dependency confusion" attacks.

If a package fails to install due to a missing build-time dependency, try using a newer version of
the package; if the problem persists, consider filing an issue with the package maintainer,
requesting that they update the packaging setup to declare the correct PEP 517 build-time
dependencies.

If uv rejects a transitive URL dependency, the best course of action is to provide the URL
dependency as a direct dependency in the relevant `pyproject.toml` or `requirement.in` file, as the
above constraints do not apply to direct dependencies.

Specifically, uv will always install packages into the currently active virtual environment, or
search for a virtual environment named `.venv` in the current directory or any parent directory
(even if it is not activated).

This differs from `pip`, which will install packages into a global environment if no virtual
environment is active, and will not search for inactive virtual environments.

In uv, you can install into non-virtual environments by providing a path to a Python executable via
the `--python /path/to/python` option, or via the `--system` flag, which installs into the first
Python interpreter found on the `PATH`, like `pip`.

In other words, uv inverts the default, requiring explicit opt-in to installing into the system
Python, which can lead to breakages and other complications, and should only be done in limited
circumstances.

For a given set of dependency specifiers, it's often the case that there is no single "correct" set
of packages to install. Instead, there are many valid sets of packages that satisfy the specifiers.

At time of writing, the most recent `starlette` version is `0.37.2`, and the most recent `fastapi`
version is `0.110.0`. However, `fastapi==0.110.0` also depends on `starlette`, and introduces an
upper bound: `starlette>=0.36.3,<0.37.0`.

When uv resolutions differ from `pip` in undesirable ways, it's often a sign that the specifiers are
too loose, and that the user should consider tightening them. For example, in the case of
`starlette` and `fastapi`, the user could require `fastapi>=0.110.0`.

Additionally, pip will fall back to the `user` install scheme if it detects that the user does not
have write permissions to the target directory, as is the case on some systems when installing into
the system Python. uv does not implement any such fallback.

The `--only-binary` argument is used to restrict installation to pre-built binary distributions.
When `--only-binary :all:` is provided, both pip and uv will refuse to build source distributions
from PyPI and other registries.

Both pip and uv allow editable requirements to be built and installed even when `--only-binary` is
provided. For example, `uv pip install -e . --only-binary :all:` is allowed.

Additionally, and in contrast to pip, uv's resolver will still read metadata from pre-built binary
distributions when `--no-binary` is provided.

In other words, if `manylinux_compatible` returns `True`, uv will treat the system as
`manylinux`-compatible; if it returns `False`, uv will treat the system as `manylinux`-incompatible,
without calling `manylinux_compatible` for every glibc version.

Kubernetes is all about sharing machines among applications.  Typically,
sharing machines requires ensuring that two applications do not try to use the
same ports.  Coordinating ports across multiple developers is very difficult to
do at scale and exposes users to cluster-level issues outside of their control.

Dynamic port allocation brings a lot of complications to the system - every
application has to take ports as flags, the API servers have to know how to
insert dynamic port numbers into configuration blocks, services have to know
how to find each other, etc.  Rather than deal with this, Kubernetes takes a
different approach.

Kubernetes clusters only consider the IP families present on the Pods, Services and Nodes objects,
independently of the existing IPs of the represented objects. For example, a server or a pod can have multiple
IP addresses assigned to its interfaces, but only the IP addresses in `node.status.addresses` or `pod.status.ips` are
considered when implementing the Kubernetes network model and defining the cluster type.

Gang scheduling ensures that a group of Pods are scheduled on an "all-or-nothing" basis.
If the cluster cannot accommodate the entire group (or a defined minimum number of Pods, specified by `minCount`),
none of the Pods are bound to a node.

The PodGroup does not enter the active scheduling queue until both conditions are met.

Ultimately, a root `CompositePodGroup` is admitted into the active scheduling queue if and only if
it satisfies the hierarchical quorum and there is at least one pending Pod that belongs to one of
its descendant `PodGroups`.

The `GangScheduling` plugin's `PlacementFeasible` method supports evaluation for both
`PodGroups` and `CompositePodGroups`. It is invoked by the scheduling cycle before starting child
evaluation and after evaluating each child group of a `CompositePodGroup`.

By taking into account the number of child groups that were successfully scheduled and the child
groups that were not evaluated in the scheduling cycle just yet, `PlacementFeasible` determines
whether the group's policy constraint is still achievable, allowing the scheduling cycle to abort
the evaluation of the `CompositePodGroup` early if its underlying scheduling policy cannot be
satisfied anymore.

A scheduler watches for newly created Pods that have no Node assigned. For
every Pod that the scheduler discovers, the scheduler becomes responsible
for finding the best Node for that Pod to run on. The scheduler reaches
this placement decision taking into account the scheduling principles
described below.

If you want to understand why Pods are placed onto a particular Node,
or if you're planning to implement a custom scheduler yourself, this
page will help you learn about scheduling.

Kube-scheduler selects an optimal node to run newly created or not yet
scheduled (unscheduled) pods. Since containers in pods - and pods themselves -
can have different requirements, the scheduler filters out any nodes that
don't meet a Pod's specific scheduling needs. Alternatively, the API lets
you specify a node for a Pod when you create it, but this is unusual
and is only done in special cases.

Factors that need to be taken into account for scheduling decisions include
individual and collective resource requirements, hardware / software /
policy constraints, affinity and anti-affinity specifications, data
locality, inter-workload interference, and so on.

Finally, kube-scheduler assigns the Pod to the Node with the highest ranking.
If there is more than one node with equal scores, kube-scheduler selects
one of these at random.

Pods were considered ready for scheduling once created. Kubernetes scheduler
does its due diligence to find nodes to place all pending Pods. However, in a
real-world case, some Pods may stay in a "miss-essential-resources" state for a long period.
These Pods actually churn the scheduler (and downstream integrators like Cluster AutoScaler)
in an unnecessary manner.

By specifying/removing a Pod's `.spec.schedulingGates`, you can control when a Pod is ready
to be considered for scheduling.

The `schedulingGates` field contains a list of strings, and each string literal is perceived as a
criteria that Pod should be satisfied before considered schedulable. This field can be initialized
only when a Pod is created (either by the client, or mutated during admission). After creation,
each schedulingGate can be removed in arbitrary order, but addition of a new scheduling gate is disallowed.

The metric `scheduler_pending_pods` comes with a new label `"gated"` to distinguish whether a Pod
has been tried scheduling but claimed as unschedulable, or explicitly marked as not ready for
scheduling. You can use `scheduler_pending_pods{queue="gated"}` to check the metric result.

IPv4/IPv6 dual-stack networking is enabled by default for your Kubernetes cluster starting in
1.21, allowing the simultaneous assignment of both IPv4 and IPv6 addresses.

The address family of a Service defaults to the address family of the first service cluster IP
range (configured via the `--service-cluster-ip-range` flag to the kube-apiserver).

If you would like to define which IP family to use for single stack or define the order of IP
families for dual-stack, you can choose the address families by setting an optional field,
`.spec.ipFamilies`, on the Service.

The first family you list is used for the legacy `.spec.clusterIP` field.

These examples demonstrate the behavior of various dual-stack Service configuration scenarios.

You can validate this behavior by using kubectl to inspect an existing service.

You can validate this behavior by using kubectl to inspect an existing headless service with selectors.

Services can be changed from single-stack to dual-stack and from dual-stack to single-stack.

Edit the Service specification updating the `.spec.ipFamilyPolicy` from `SingleStack` to `PreferDualStack`.

Kubernetes on Windows does not support single-stack "IPv6-only" networking. However,
dual-stack IPv4/IPv6 networking for pods and nodes with single-family services
is supported.

You can use IPv4/IPv6 dual-stack networking with `l2bridge` networks.

Some applications need additional storage but don't care whether that
data is stored persistently across restarts. For example, caching
services are often limited by memory size and can move infrequently
used data into storage that is slower than memory with little impact
on overall performance.

Other applications expect some read-only input data to be present in
files, like configuration data or secret keys.

The advantage of using third-party drivers is that they can offer
functionality that Kubernetes itself does not support, for example
storage with different performance characteristics than the disk that
is managed by kubelet, or injecting different data.

The `volumeAttributes` determine what volume is prepared by the
driver. These attributes are specific to each driver and not
standardized. See the documentation of each CSI driver for further
instructions.

CSI ephemeral volumes allow users to provide `volumeAttributes`
directly to the CSI driver as part of the Pod spec. A CSI driver
allowing `volumeAttributes` that are typically restricted to
administrators is NOT suitable for use in an inline ephemeral volume.
For example, parameters that are normally defined in the StorageClass
should not be exposed to users through the use of inline ephemeral volumes.

While these PVCs exist, they can be used like any other PVC. In
particular, they can be referenced as data source in volume cloning or
snapshotting. The PVC object also holds the current status of the
volume.

Naming of the automatically created PVCs is deterministic: the name is
a combination of the Pod name and volume name, with a hyphen (`-`) in the
middle. In the example above, the PVC name will be
`my-app-scratch-volume`.  This deterministic naming makes it easier to
interact with the PVC because one does not have to search for it once
the Pod name and volume name are known.

The deterministic naming also introduces a potential conflict between different
Pods (a Pod "pod-a" with volume "scratch" and another Pod with name
"pod" and volume "a-scratch" both end up with the same PVC name
"pod-a-scratch") and between Pods and manually created PVCs.

Such conflicts are detected: a PVC is only used for an ephemeral
volume if it was created for the Pod. This check is based on the
ownership relationship. An existing PVC is not overwritten or
modified. But this does not resolve the conflict because without the
right PVC, the Pod cannot start.

If the resource usage decreases, and the Pod resource requests are above optimal levels, 
the VerticalPodAutoscaler instructs the workload resource (the Deployment, StatefulSet, or other similar resource) 
to adjust resource requests back down, preventing resource waste.

Once during each period, the Recommender queries the resource utilization for Pods targeted by each VerticalPodAutoscaler definition. The Recommender finds the target resource defined by the `targetRef`, then selects the pods based on the target resource's `.spec.selector` labels, and obtains the metrics from the resource metrics API to analyze actual CPU and memory consumption.

These recommendations are stored in the VerticalPodAutoscaler resource's `.status.recommendation` field.

The chosen method depends on the configured update mode, cluster capabilities, and the type of resource change needed. In-place updates, when available, avoid Pod disruption but may have limitations on which resources can be modified. The updater respects PodDisruptionBudgets to minimize service impact.

You can use a tool such as `kubectl` to view the `.status` and the recommendations in it.

In `InPlaceOrRecreate` mode, VPA attempts to update Pod resource requests and limits without restarting the Pod when possible. However, if in-place updates cannot be performed for a particular resource change, VPA falls back to evicting the Pod
(similar to `Recreate` mode) and allowing the workload controller to create a replacement Pod with updated resources.

Resource policies allow you to fine-tune how the VerticalPodAutoscaler generates recommendations and applies updates.
You can set boundaries for resource recommendations, specify which resources to manage, and configure different policies for individual containers within a Pod.

These fields set boundaries for VPA recommendations.
The VPA will never recommend resources below `minAllowed` or above `maxAllowed`, even if the actual usage data suggests different values.

The `controlledResources` field specifies which resource types VPA should manage for a container in a Pod.
If not specified, VPA manages both CPU and memory by default. You can restrict VPA to manage only specific resources.
Valid resource names include `cpu` and `memory`.

RequestsAndLimits
: VPA sets both requests and limits. The limit scales proportionally to the request based on the request-to-limit ratio defined in the Pod spec. This is the default mode.

RequestsOnly
: VPA only sets requests, leaving limits unchanged. Limits are respected and can still trigger throttling or out-of-memory kills if usage exceeds them.

For example, if the `max` field in a Container LimitRange resource is exceeded, both VPA components lower the limit to the value defined in the `max` field, and the request is proportionally decreased to maintain the request-to-limit ratio in the Pod spec.

If there are too many pods, the ReplicationController terminates the extra pods. If there are too few, the
ReplicationController starts more pods. Unlike manually created pods, the pods maintained by a
ReplicationController are automatically replaced if they fail, are deleted, or are terminated.
For example, your pods are re-created on a node after disruptive maintenance such as a kernel upgrade.
For this reason, you should use a ReplicationController even if your application requires
only a single pod. A ReplicationController is similar to a process supervisor,
but instead of supervising individual processes on a single node, the ReplicationController supervises multiple pods
across multiple nodes.

ReplicationController is often abbreviated to "rc" in discussion, and as a shortcut in
kubectl commands.

A simple case is to create one ReplicationController object to reliably run one instance of
a Pod indefinitely.  A more complex use case is to run several identical replicas of a replicated
service, such as web servers.

This example ReplicationController config runs three copies of the nginx web server.

Here, the selector is the same as the selector for the ReplicationController (seen in the
`kubectl describe` output), and in a different form in `replication.yaml`.  The `--output=jsonpath` option
specifies an expression with the name from each pod in the returned list.

As with all other Kubernetes config, a ReplicationController needs `apiVersion`, `kind`, and `metadata` fields.

The `.spec.template` is the only required field of the `.spec`.

The ReplicationController can itself have labels (`.metadata.labels`).  Typically, you
would set these the same as the `.spec.template.metadata.labels`; if `.metadata.labels` is not specified
then it defaults to  `.spec.template.metadata.labels`. However, they are allowed to be
different, and the `.metadata.labels` do not affect the behavior of the ReplicationController.

If specified, the `.spec.template.metadata.labels` must be equal to the `.spec.selector`, or it will
be rejected by the API.  If `.spec.selector` is unspecified, it will be defaulted to
`.spec.template.metadata.labels`.

Also you should not normally create any pods whose labels match this selector, either directly, with
another ReplicationController, or with another controller such as Job. If you do so, the
ReplicationController thinks that it created the other pods.  Kubernetes does not stop you
from doing this.

You can specify how many pods should run concurrently by setting `.spec.replicas` to the number
of pods you would like to have running concurrently.  The number running at any time may be higher
or lower, such as if the replicas were just increased or decreased, or if a pod is gracefully
shutdown, and a replacement starts early.

If you do not specify `.spec.replicas`, then it defaults to 1.

You can delete a ReplicationController without affecting any of its pods.

Pods may be removed from a ReplicationController's target set by changing their labels. This technique may be used to remove pods from service for debugging and data recovery. Pods that are removed in this way will be replaced automatically (assuming that the number of replicas is not also changed).

As mentioned above, whether you have 1 pod you want to keep running, or 1000, a ReplicationController will ensure that the specified number of pods exists, even in the event of node failure or pod termination (for example, due to an action by another control agent).

The ReplicationController enables scaling the number of replicas up or down, either manually or by an auto-scaling control agent, by updating the `replicas` field.

The ReplicationController is designed to facilitate rolling updates to a service by replacing pods one-by-one.

Ideally, the rolling update controller would take application readiness into account, and would ensure that a sufficient number of pods were productively serving at any given time.

The two ReplicationControllers would need to create pods with at least one differentiating label, such as the image tag of the primary container of the pod, since it is typically image updates that motivate rolling updates.

In addition to running multiple releases of an application while a rolling update is in progress, it's common to run multiple releases for an extended period of time, or even continuously, using multiple release tracks. The tracks would be differentiated by labels.

For instance, a service might target all pods with `tier in (frontend), environment in (prod)`.  Now say you have 10 replicated pods that make up this tier.  But you want to be able to 'canary' a new version of this component.  You could set up a ReplicationController with `replicas` set to 9 for the bulk of the replicas, with labels `tier=frontend, environment=prod, track=stable`, and another ReplicationController with `replicas` set to 1 for the canary, with labels `tier=frontend, environment=prod, track=canary`.  Now the service is covering both the canary and non-canary pods.  But you can mess with the ReplicationControllers separately to test things out, monitor the results, etc.

Multiple ReplicationControllers can sit behind a single service, so that, for example, some traffic
goes to the old version, and some goes to the new version.

A ReplicationController will never terminate on its own, but it isn't expected to be as long-lived as services. Services may be composed of pods controlled by multiple ReplicationControllers, and it is expected that many ReplicationControllers may be created and destroyed over the lifetime of a service (for instance, to perform an update of pods that run the service). Both services themselves and their clients should remain oblivious to the ReplicationControllers that maintain the pods of the services.

Unlike in the case where a user directly created pods, a ReplicationController replaces pods that are deleted or terminated for any reason, such as in the case of node failure or disruptive node maintenance, such as a kernel upgrade. For this reason, we recommend that you use a ReplicationController even if your application requires only a single pod. Think of it similarly to a process supervisor, only it supervises multiple pods across multiple nodes instead of individual processes on a single node.  A ReplicationController delegates local container restarts to some agent on the node, such as the kubelet.

Each controller chooses the field names and structure that are idiomatic for its own API, so
these blocks impose no fixed top-level shape. Although not strictly enforced, reusing the 
standard field names and structure is recommended, so that a user who has configured gang 
scheduling on one controller's resource recognizes the same options on another's.

A different controller might nest the same blocks per component of a multi-part workload,
or support only a subset of them.

The one difference is that the block's `gang` minimum count is optional. Users may leave
`minCount` unset, in which case the controller supplies a default that makes sense for its
own domain; the Job controller, for example, uses the Job's parallelism.

The library rejects combinations that are not meaningful. For example, prevent `all` disruption mode 
for PodGroups with BasicSchedulingPolicy, because the preemption unit must not be larger than the 
scheduling unit - a group scheduled pod-by-pod has no group-level unit to preempt or disrupt.

Pods consume the devices allocated to the group by declaring a matching claim in their own
spec, using the same name and referring to the same object.

The library consumes the `scheduling.k8s.io/v1alpha3` building blocks and compiles them into
`scheduling.k8s.io/v1beta1` Workload and PodGroup objects, while CompositePodGroup
objects remain `scheduling.k8s.io/v1alpha3`.

Validation also differs between creating and updating an object. On an update, the library
additionally enforces the fields that are frozen after creation, which means the controller
has to supply the previously stored configuration along with the new one. On a create there is
nothing to compare against, so those checks do not apply.

Because the building-block types are shared across controllers, future releases may add
scheduling options that do not make sense for every controller. To keep new options from
silently leaking in, `workloadbuilder` uses an allow-list model: a controller declares the
policies and disruption modes it supports, and `Validate` rejects anything outside that set,
reporting the error at the offending block's field path.

Options are therefore denied by default. When a new policy is introduced, an existing
controller keeps rejecting it until its maintainers extend the allow-list, which for an
out-of-tree controller means updating its vendored copy of the library as well.

When the Workload already exists, whether compiled by a parent controller or created manually,
a child controller that only manages the runtime PodGroup uses `NewBuilderFromExistingWorkload`
instead. That builder creates PodGroup objects from the supplied Workload using its own owner
reference. It does not validate or compile anything, so the existing Workload is never
recompiled.

An alias is an alternative name for a field, used when serializing and deserializing data.

Pydantic provides two special types for convenience when using `validation_alias`: `AliasPath` and `AliasChoices`.

Unions are fundamentally different to all other types Pydantic validates - instead of requiring all fields/items/values to be valid, unions require only one member to be valid.

Validating unions feels like adding another orthogonal dimension to the validation process.

For complex cases, if you're using untagged unions, it's recommended to use `union_mode='left_to_right'` if you need guarantees about the order of validation attempts against the union members.

With this approach, validation is attempted against each member of the union in their order they're defined, and the first successful validation is accepted as input.

If validation fails on all members, the validation error includes the errors from all members of the union.

Rust is ideal for many people for a variety of reasons. Let’s look at a few of
the most important groups.

Rust is proving to be a productive tool for collaborating among large teams of
developers with varying levels of systems programming knowledge. Low-level code
is prone to various subtle bugs, which in most other languages can only be
caught through extensive testing and careful code review by experienced
developers. In Rust, the compiler plays a gatekeeper role by refusing to
compile code with these elusive bugs, including concurrency bugs. By working
alongside the compiler, the team can spend its time focusing on the program’s
logic rather than chasing down bugs.

By using these and other tools in the Rust ecosystem, developers can be
productive while writing systems-level code.

Rust is for students and those who are interested in learning about systems
concepts. Using Rust, many people have learned about topics like operating
systems development. The community is very welcoming and happy to answer
students’ questions. Through efforts such as this book, the Rust teams want to
make systems concepts more accessible to more people, especially those new to
programming.

Hundreds of companies, large and small, use Rust in production for a variety of
tasks, including command line tools, web services, DevOps tooling, embedded
devices, audio and video analysis and transcoding, cryptocurrencies,
bioinformatics, search engines, Internet of Things applications, machine
learning, and even major parts of the Firefox web browser.

Rust is for people who want to build the Rust programming language, community,
developer tools, and libraries. We’d love to have you contribute to the Rust
language.

Rust is for people who crave speed and stability in a language. By speed, we
mean both how quickly Rust code can run and the speed at which Rust lets you
write programs. The Rust compiler’s checks ensure stability through feature
additions and refactoring. This is in contrast to the brittle legacy code in
languages without these checks, which developers are often afraid to modify. By
striving for zero-cost abstractions—higher-level features that compile to
lower-level code as fast as code written manually—Rust endeavors to make safe
code be fast code as well.

In general, this book assumes that you’re reading it in sequence from front to
back. Later chapters build on concepts in earlier chapters, and earlier
chapters might not delve into details on a particular topic but will revisit
the topic in a later chapter.

You’ll find two kinds of chapters in this book: concept chapters and project
chapters. In concept chapters, you’ll learn about an aspect of Rust. In project
chapters, we’ll build small programs together, applying what you’ve learned so
far. Chapter 2, Chapter 12, and Chapter 21 are project chapters; the rest are
concept chapters.

There is no wrong way to read this book: If you want to skip ahead, go for it!
You might have to jump back to earlier chapters if you experience any
confusion. But do whatever works for you.

In most situations, we’ll lead you to the correct version of any code that
doesn’t compile.

To understand when we might want to use structs, let’s write a program that
calculates the area of a rectangle. We’ll start by using single variables and
then refactor the program until we’re using structs instead.

This code succeeds in figuring out the area of the rectangle by calling the
`area` function with each dimension, but we can do more to make this code clear
and readable.

Listing 5-9 shows another version of our program that uses tuples.

In one way, this program is better. Tuples let us add a bit of structure, and
we’re now passing just one argument. But in another way, this version is less
clear: Tuples don’t name their elements, so we have to index into the parts of
the tuple, making our calculation less obvious.

Mixing up the width and height wouldn’t matter for the area calculation, but if
we want to draw the rectangle on the screen, it would matter! We would have to
keep in mind that `width` is the tuple index `0` and `height` is the tuple
index `1`. This would be even harder for someone else to figure out and keep in
mind if they were to use our code. Because we haven’t conveyed the meaning of
our data in our code, it’s now easier to introduce errors.

We use structs to add meaning by labeling the data. We can transform the tuple
we’re using into a struct with a name for the whole as well as names for the
parts, as shown in Listing 5-10.

Here, we’ve defined a struct and named it `Rectangle`. Inside the curly
brackets, we defined the fields as `width` and `height`, both of which have
type `u32`. Then, in `main`, we created a particular instance of `Rectangle`
that has a width of `30` and a height of `50`.

Our `area` function is now defined with one parameter, which we’ve named
`rectangle`, whose type is an immutable borrow of a struct `Rectangle`
instance. As mentioned in Chapter 4, we want to borrow the struct rather than
take ownership of it. This way, `main` retains its ownership and can continue
using `rect1`, which is the reason we use the `&` in the function signature and
where we call the function.

The `area` function accesses the `width` and `height` fields of the `Rectangle`
instance (note that accessing fields of a borrowed struct instance does not
move the field values, which is why you often see borrows of structs). Our
function signature for `area` now says exactly what we mean: Calculate the area
of `Rectangle`, using its `width` and `height` fields. This conveys that the
width and height are related to each other, and it gives descriptive names to
the values rather than using the tuple index values of `0` and `1`. This is a
win for clarity.

Our `area` function is very specific: It only computes the area of rectangles.
It would be helpful to tie this behavior more closely to our `Rectangle` struct
because it won’t work with any other type. Let’s look at how we can continue to
refactor this code by turning the `area` function into an `area` method
defined on our `Rectangle` type.

Sometimes bad things happen in your code, and there’s nothing you can do about
it. In these cases, Rust has the `panic!` macro. There are two ways to cause a
panic in practice: by taking an action that causes our code to panic (such as
accessing an array past the end) or by explicitly calling the `panic!` macro.
In both cases, we cause a panic in our program. By default, these panics will
print a failure message, unwind, clean up the stack, and quit. Via an
environment variable, you can also have Rust display the call stack when a
panic occurs to make it easier to track down the source of the panic.

In this case, the line indicated is part of our code, and if we go to that
line, we see the `panic!` macro call. In other cases, the `panic!` call might
be in code that our code calls, and the filename and line number reported by
the error message will be someone else’s code where the `panic!` macro is
called, not the line of our code that eventually led to the `panic!` call.

We can use the backtrace of the functions the `panic!` call came from to figure
out the part of our code that is causing the problem. To understand how to use
a `panic!` backtrace, let’s look at another example and see what it’s like when
a `panic!` call comes from a library because of a bug in our code instead of
from our code calling the macro directly. Listing 9-1 has some code that
attempts to access an index in a vector beyond the range of valid indexes.

Here, we’re attempting to access the 100th element of our vector (which is at
index 99 because indexing starts at zero), but the vector has only three
elements. In this situation, Rust will panic. Using `[]` is supposed to return
an element, but if you pass an invalid index, there’s no element that Rust
could return here that would be correct.

That’s a lot of output! The exact output you see might be different depending
on your operating system and Rust version. In order to get backtraces with this
information, debug symbols must be enabled. Debug symbols are enabled by
default when using `cargo build` or `cargo run` without the `--release` flag,
as we have here.

We’ll improve the `minigrep` binary by adding an extra feature: an option for
case-insensitive searching that the user can turn on via an environment
variable. We could make this feature a command line option and require that
users enter it each time they want it to apply, but by instead making it an
environment variable, we allow our users to set the environment variable once
and have all their searches be case insensitive in that terminal session.

We first add a new `search_case_insensitive` function to the `minigrep` library
that will be called when the environment variable has a value. We’ll continue
to follow the TDD process, so the first step is again to write a failing test.
We’ll add a new test for the new `search_case_insensitive` function and rename
our old test from `one_result` to `case_sensitive` to clarify the differences
between the two tests, as shown in Listing 12-20.

The `search_case_insensitive` function, shown in Listing 12-21, will be almost
the same as the `search` function. The only difference is that we’ll lowercase
the `query` and each `line` so that whatever the case of the input arguments,
they’ll be the same case when we check whether the line contains the query.

First, we lowercase the `query` string and store it in a new variable with the
same name, shadowing the original `query`. Calling `to_lowercase` on the query
is necessary so that no matter whether the user’s query is `"rust"`, `"RUST"`,
`"Rust"`, or `"rUsT"`, we’ll treat the query as if it were `"rust"` and be
insensitive to the case. While `to_lowercase` will handle basic Unicode, it
won’t be 100 percent accurate. If we were writing a real application, we’d want
to do a bit more work here, but this section is about environment variables,
not Unicode, so we’ll leave it at that here.

Note that `query` is now a `String` rather than a string slice because calling
`to_lowercase` creates new data rather than referencing existing data. Say the
query is `"rUsT"`, as an example: That string slice doesn’t contain a lowercase
`u` or `t` for us to use, so we have to allocate a new `String` containing
`"rust"`. When we pass `query` as an argument to the `contains` method now, we
need to add an ampersand because the signature of `contains` is defined to take
a string slice.

Next, we add a call to `to_lowercase` on each `line` to lowercase all
characters. Now that we’ve converted `line` and `query` to lowercase, we’ll
find matches no matter what the case of the query is.

We added the `ignore_case` field that holds a Boolean. Next, we need the `run`
function to check the `ignore_case` field’s value and use that to decide
whether to call the `search` function or the `search_case_insensitive`
function, as shown in Listing 12-22. This still won’t compile yet.

Here, we create a new variable, `ignore_case`. To set its value, we call the
`env::var` function and pass it the name of the `IGNORE_CASE` environment
variable. The `env::var` function returns a `Result` that will be the
successful `Ok` variant that contains the value of the environment variable if
the environment variable is set to any value. It will return the `Err` variant
if the environment variable is not set.

We pass the value in the `ignore_case` variable to the `Config` instance so
that the `run` function can read that value and decide whether to call
`search_case_insensitive` or `search`, as we implemented in Listing 12-22.

The `std::env` module contains many more useful features for dealing with
environment variables: Check out its documentation to see what is available.

We’ll talk about each of these in turn, but first, let’s look at why we even
need macros when we already have functions.

Metaprogramming is useful for reducing the amount of code you have to write and
maintain, which is also one of the roles of functions. However, macros have
some additional powers that functions don’t have.

A function signature must declare the number and type of parameters the
function has. Macros, on the other hand, can take a variable number of
parameters: We can call `println!("hello")` with one argument or
`println!("hello {}", name)` with two arguments. Also, macros are expanded
before the compiler interprets the meaning of the code, so a macro can, for
example, implement a trait on a given type. A function can’t, because it gets
called at runtime and a trait needs to be implemented at compile time.

The downside to implementing a macro instead of a function is that macro
definitions are more complex than function definitions because you’re writing
Rust code that writes Rust code. Due to this indirection, macro definitions are
generally more difficult to read, understand, and maintain than function
definitions.

We could also use the `vec!` macro to make a vector of two integers or a vector
of five string slices. We wouldn’t be able to use a function to do the same
because we wouldn’t know the number or type of values up front.

Listing 20-35 shows a slightly simplified definition of the `vec!` macro.

The `#[macro_export]` annotation indicates that this macro should be made
available whenever the crate in which the macro is defined is brought into
scope. Without this annotation, the macro can’t be brought into scope.

The structure in the `vec!` body is similar to the structure of a `match`
expression. Here we have one arm with the pattern `( $( $x:expr ),* )`,
followed by `=>` and the block of code associated with this pattern. If the
pattern matches, the associated block of code will be emitted. Given that this
is the only pattern in this macro, there is only one valid way to match; any
other pattern will result in an error. More complex macros will have more than
one arm.

First, we use a set of parentheses to encompass the whole pattern. We use a
dollar sign (`$`) to declare a variable in the macro system that will contain
the Rust code matching the pattern. The dollar sign makes it clear this is a
macro variable as opposed to a regular Rust variable. Next comes a set of
parentheses that captures values that match the pattern within the parentheses
for use in the replacement code. Within `$()` is `$x:expr`, which matches any
Rust expression and gives the expression the name `$x`.

The comma following `$()` indicates that a literal comma separator character
must appear between each instance of the code that matches the code in `$()`.
The `*` specifies that the pattern matches zero or more of whatever precedes
the `*`.

When we call this macro with `vec![1, 2, 3];`, the `$x` pattern matches three
times with the three expressions `1`, `2`, and `3`.

We’ve defined a macro that can take any number of arguments of any type and can
generate code to create a vector containing the specified elements.

When creating procedural macros, the definitions must reside in their own crate
with a special crate type. This is for complex technical reasons that we hope
to eliminate in the future. In Listing 20-36, we show how to define a
procedural macro, where `some_attribute` is a placeholder for using a specific
macro variety.

Next, in Listing 20-38, we’ll define the `HelloMacro` trait and its associated
function.

We have a trait and its function. At this point, our crate user could implement
the trait to achieve the desired functionality, as in Listing 20-39.

However, they would need to write the implementation block for each type they
wanted to use with `hello_macro`; we want to spare them from having to do this
work.

Additionally, we can’t yet provide the `hello_macro` function with default
implementation that will print the name of the type the trait is implemented
on: Rust doesn’t have reflection capabilities, so it can’t look up the type’s
name at runtime. We need a macro to generate code at compile time.

Our two crates are tightly related, so we create the procedural macro crate
within the directory of our `hello_macro` crate. If we change the trait
definition in `hello_macro`, we’ll have to change the implementation of the
procedural macro in `hello_macro_derive` as well. The two crates will need to
be published separately, and programmers using these crates will need to add
both as dependencies and bring them both into scope. We could instead have the
`hello_macro` crate use `hello_macro_derive` as a dependency and re-export the
procedural macro code. However, the way we’ve structured the project makes it
possible for programmers to use `hello_macro` even if they don’t want the
`derive` functionality.

Notice that we’ve split the code into the `hello_macro_derive` function, which
is responsible for parsing the `TokenStream`, and the `impl_hello_macro`
function, which is responsible for transforming the syntax tree: This makes
writing a procedural macro more convenient. The code in the outer function
(`hello_macro_derive` in this case) will be the same for almost every
procedural macro crate you see or create. The code you specify in the body of
the inner function (`impl_hello_macro` in this case) will be different
depending on your procedural macro’s purpose.

The `syn` crate parses Rust code from a string into a data structure that we
can perform operations on. The `quote` crate turns `syn` data structures back
into Rust code. These crates make it much simpler to parse any sort of Rust
code we might want to handle: Writing a full parser for Rust code is no simple
task.

The `hello_macro_derive` function will be called when a user of our library
specifies `#[derive(HelloMacro)]` on a type. This is possible because we’ve
annotated the `hello_macro_derive` function here with `proc_macro_derive` and
specified the name `HelloMacro`, which matches our trait name; this is the
convention most procedural macros follow.

The `hello_macro_derive` function first converts the `input` from a
`TokenStream` to a data structure that we can then interpret and perform
operations on. This is where `syn` comes into play. The `parse` function in
`syn` takes a `TokenStream` and returns a `DeriveInput` struct representing the
parsed Rust code. Listing 20-41 shows the relevant parts of the `DeriveInput`
struct we get from parsing the `struct Pancakes;` string.

We get an `Ident` struct instance containing the name (identifier) of the
annotated type using `ast.ident`. The struct in Listing 20-41 shows that when
we run the `impl_hello_macro` function on the code in Listing 20-37, the
`ident` we get will have the `ident` field with a value of `"Pancakes"`. Thus,
the `name` variable in Listing 20-42 will contain an `Ident` struct instance
that, when printed, will be the string `"Pancakes"`, the name of the struct in
Listing 20-37.

The `quote!` macro lets us define the Rust code that we want to return. The
compiler expects something different from the direct result of the `quote!`
macro’s execution, so we need to convert it to a `TokenStream`. We do this by
calling the `into` method, which consumes this intermediate representation and
returns a value of the required `TokenStream` type.

We want our procedural macro to generate an implementation of our `HelloMacro`
trait for the type the user annotated, which we can get by using `#name`. The
trait implementation has the one function `hello_macro`, whose body contains the
functionality we want to provide: printing `Hello, Macro! My name is` and then
the name of the annotated type.

The `stringify!` macro used here is built into Rust. It takes a Rust
expression, such as `1 + 2`, and at compile time turns the expression into a
string literal, such as `"1 + 2"`. This is different from `format!` or
`println!`, which are macros that evaluate the expression and then turn the
result into a `String`. There is a possibility that the `#name` input might be
an expression to print literally, so we use `stringify!`. Using `stringify!`
also saves an allocation by converting `#name` to a string literal at compile
time.

Next, let’s explore how the other kinds of procedural macros differ from custom
`derive` macros.

Here, we have two parameters of type `TokenStream`. The first is for the
contents of the attribute: the `GET, "/"` part. The second is the body of the
item the attribute is attached to: in this case, `fn index() {}` and the rest
of the function’s body.

Other than that, attribute-like macros work the same way as custom `derive`
macros: You create a crate with the `proc-macro` crate type and implement a
function that generates the code you want!

This definition is similar to the custom `derive` macro’s signature: We receive
the tokens that are inside the parentheses and return the code we wanted to
generate.

Whew! Now you have some Rust features in your toolbox that you likely won’t use
often, but you’ll know they’re available in very particular circumstances.
We’ve introduced several complex topics so that when you encounter them in
error message suggestions or in other people’s code, you’ll be able to
recognize these concepts and syntax. Use this chapter as a reference to guide
you to solutions.

Next, we’ll put everything we’ve discussed throughout the book into practice
and do one more project!

We’ll start by getting a single-threaded web server working. Before we begin,
let’s look at a quick overview of the protocols involved in building web
servers. The details of these protocols are beyond the scope of this book, but
a brief overview will give you the information you need.

TCP is the lower-level protocol that describes the details of how information
gets from one server to another but doesn’t specify what that information is.
HTTP builds on top of TCP by defining the contents of the requests and
responses. It’s technically possible to use HTTP with other protocols, but in
the vast majority of cases, HTTP sends its data over TCP. We’ll work with the
raw bytes of TCP and HTTP requests and responses.

The `bind` function returns a `Result<T, E>`, which indicates that it’s
possible for binding to fail, for example, if we ran two instances of our
program and so had two programs listening to the same port. Because we’re
writing a basic server just for learning purposes, we won’t worry about
handling these kinds of errors; instead, we use `unwrap` to stop the program if
errors happen.

It could also be that the browser is trying to connect to the server multiple
times because the server isn’t responding with any data. When `stream` goes out
of scope and is dropped at the end of the loop, the connection is closed as
part of the `drop` implementation. Browsers sometimes deal with closed
connections by retrying, because the problem might be temporary.

The important factor is that we’ve successfully gotten a handle to a TCP
connection!

Let’s implement the functionality to read the request from the browser! To
separate the concerns of first getting a connection and then taking some action
with the connection, we’ll start a new function for processing connections. In
this new `handle_connection` function, we’ll read data from the TCP stream and
print it so that we can see the data being sent from the browser. Change the
code to look like Listing 21-2.

We bring `std::io::BufReader` and `std::io::prelude` into scope to get access
to traits and types that let us read from and write to the stream. In the `for`
loop in the `main` function, instead of printing a message that says we made a
connection, we now call the new `handle_connection` function and pass the
`stream` to it.

In the `handle_connection` function, we create a new `BufReader` instance that
wraps a reference to the `stream`. The `BufReader` adds buffering by managing
calls to the `std::io::Read` trait methods for us.

We create a variable named `http_request` to collect the lines of the request
the browser sends to our server. We indicate that we want to collect these
lines in a vector by adding the `Vec<_>` type annotation.

The browser signals the end of an HTTP request by sending two newline
characters in a row, so to get one request from the stream, we take lines until
we get a line that is the empty string. Once we’ve collected the lines into the
vector, we’re printing them out using pretty debug formatting so that we can
take a look at the instructions the web browser is sending to our server.

Let’s break down this request data to understand what the browser is asking of
our program.

After the request line, the remaining lines starting from `Host:` onward are
headers. `GET` requests have no body.

Now that we know what the browser is asking for, let’s send back some data!

The status code 200 is the standard success response. The text is a tiny
successful HTTP response. Let’s write this to the stream as our response to a
successful request! From the `handle_connection` function, remove the
`println!` that was printing the request data and replace it with the code in
Listing 21-3.

The first new line defines the `response` variable that holds the success
message’s data. Then, we call `as_bytes` on our `response` to convert the
string data to bytes. The `write_all` method on `stream` takes a `&[u8]` and
sends those bytes directly down the connection. Because the `write_all`
operation could fail, we use `unwrap` on any error result as before. Again, in
a real application, you would add error handling here.

This is a minimal HTML5 document with a heading and some text. To return this
from the server when a request is received, we’ll modify `handle_connection` as
shown in Listing 21-5 to read the HTML file, add it to the response as a body,
and send it.

We’ve added `fs` to the `use` statement to bring the standard library’s
filesystem module into scope. The code for reading the contents of a file to a
string should look familiar; we used it when we read the contents of a file for
our I/O project in Listing 12-4.

Next, we use `format!` to add the file’s contents as the body of the success
response. To ensure a valid HTTP response, we add the `Content-Length` header,
which is set to the size of our response body—in this case, the size of
`hello.html`.

We’re only going to be looking at the first line of the HTTP request, so rather
than reading the entire request into a vector, we’re calling `next` to get the
first item from the iterator. The first `unwrap` takes care of the `Option` and
stops the program if the iterator has no items. The second `unwrap` handles the
`Result` and has the same effect as the `unwrap` that was in the `map` added in
Listing 21-2.

Now let’s add the code in Listing 21-7 to the `else` block to return a response
with the status code 404, which signals that the content for the request was
not found. We’ll also return some HTML for a page to render in the browser
indicating the response to the end user.

At the moment, the `if` and `else` blocks have a lot of repetition: They’re
both reading files and writing the contents of the files to the stream. The
only differences are the status line and the filename. Let’s make the code more
concise by pulling out those differences into separate `if` and `else` lines
that will assign the values of the status line and the filename to variables;
we can then use those variables unconditionally in the code to read the file
and write the response. Listing 21-9 shows the resultant code after replacing
the large `if` and `else` blocks.

Now the `if` and `else` blocks only return the appropriate values for the
status line and filename in a tuple; we then use destructuring to assign these
two values to `status_line` and `filename` using a pattern in the `let`
statement, as discussed in Chapter 19.

The previously duplicated code is now outside the `if` and `else` blocks and
uses the `status_line` and `filename` variables. This makes it easier to see
the difference between the two cases, and it means we have only one place to
update the code if we want to change how the file reading and response writing
work. The behavior of the code in Listing 21-9 will be the same as that in
Listing 21-7.

Awesome! We now have a simple web server in approximately 40 lines of Rust code
that responds to one request with a page of content and responds to all other
requests with a 404 response.

Currently, our server runs in a single thread, meaning it can only serve one
request at a time. Let’s examine how that can be a problem by simulating some
slow requests. Then, we’ll fix it so that our server can handle multiple
requests at once.

Pass `--sync` to install the selected packages before collecting module ownership information.
Synchronization preserves unrelated installed packages by default; add `--exact` to remove them.

The primary structure is the "resolution" field which contains the dependency graph with exact
package versions that a `uv.lock` encodes.

Package-derived nodes in the graph are uniquely identified by package `name`, `version`, `source`,
and `kind`. Script and workspace nodes are identified by their path. Workspace-root dependency group
nodes are identified by their group name and the workspace path. All node ids should be treated as
opaque.

If you want to install `mypackage`, find its `"kind": "package"` node. This node will also include
information on its sdist, its wheels, its extras (`optional_dependencies`), and dependency groups
(`dependency_groups`).

If you want to install `mypackage[myextra]` then find the node with `"kind": { "extra": "myextra" }`
for `mypackage` (this node will always depend on `mypackage`). If you want to install
`mypackage[extra1, extra2]`, find the two nodes for `mypackage[extra1]` and `mypackage[extra2]`.

If the workspace root defines dependency groups but is not itself a package, its `"workspace"` node
provides the corresponding group node ids through `dependency_groups`.

When an environment exists, `environment.packages` is an inventory of its installed distributions,
including those absent from the lockfile. Each entry contains the installed name, version, metadata
path, and editable status.

An installed distribution is matched by normalized name to a package in the selected resolution. A
matched entry then uses that package's resolution node ID. The entry's `version` reports the actual
installed version; node IDs should be treated as opaque.

Two versions of a package cannot be installed into a Python environment, but the dependency graph
may still include multiple versions of a package. This can happen for two different reasons.

If you just want to get "every version of pydantic this workspace uses" you're free to iterate
through the list of nodes and collect up every instance. If however you want to specifically analyze
the graph and get actual resolutions you will likely need to consult `conflicts` and need to
understand how to resolve `markers` for a specific platform.

The best way to avoid mistakes when working with multiple versions of a package is to keep your
queries into the dependency graph rooted in operations on the workspace root, workspace members, or
the requested script. These are the natural entry-points to the graph and can give coherent
responses for operations such as "install the workspace `dev` group", "install `member1` and
`member2[extra]`", or "install this script's declared dependencies".

So rather than trying to find a node for anyio in the dependency graph directly, you should decide
what workspace member(s) you're interested in analyzing as if they were going to be installed. While
traversing the `dependencies` of the things you want to install, you may visit an instance of anyio,
which is the one you should use. If you visit multiple instances of anyio then that means you've
selected a conflicting set of things to install which uv would never select.

Since this build failure occurs in pip too, it is not likely to be a bug with uv.

If a build failure is reproducible with another installer, you should investigate upstream (in this
example, `numpy` or `setuptools`), find a way to avoid building the package in the first place, or
make the necessary adjustments to your system for the build to succeed.

When generating the cross-platform lockfile, uv needs to determine the dependencies of all packages,
even those only installed on other platforms. uv tries to avoid package builds during resolution. It
uses a wheel if one exists for that version, then tries to find static metadata in the source
distribution (mainly pyproject.toml with static `project.version`, `project.dependencies` and
`project.optional-dependencies` or METADATA v2.2+). If both approaches fail, uv builds the package.

When installing, uv needs to have a wheel for the current platform for each package. If no matching
wheel exists in the index, uv tries to build the source distribution.

The following examples demonstrate common build failures and how to resolve them.

If the build error mentions a missing header or library, e.g., a `.h` file, then you'll need to
install it with your system package manager.

Note that installing the `graphviz` package is not sufficient, the development headers need to be
installed.

For example, when resolving the following dependencies on Python 3.10, uv attempts to build an old
version of `apache-beam`.

The build constraint will thus ensure that any package requiring `setuptools` during the build
process will avoid using the problematic version, preventing build failures caused by incompatible
build dependencies.

A minimal reproducible example (MRE) is essential for fixing bugs. Without an example that can be
used to reproduce the problem, a maintainer cannot debug it or test if it is fixed. If the example
is not minimal, i.e., if it includes lots of content which is not related to the issue, it can take
a maintainer much longer to identify the root cause of the problem.

Writing a Docker image is often the best way to share a reproducible example because it is entirely
self-contained. This means that the state from the reproducer's system does not affect the problem.

Whenever a script relies on external state, be sure to share that information. For example, if you
wrote the script on Windows, and it uses a Python version that you installed with `choco` and runs
on PowerShell 6.2, please include that in the report.

FastAPI is a modern, fast (high-performance), web framework for building APIs with Python based on standard Python type hints.

Now modify the file `main.py` to receive a body from a `PUT` request.

Declare the body using standard Python types, thanks to Pydantic.

We just scratched the surface, but you already get the idea of how it all works.

Whether you're selecting files, browsing command history, previewing data,
navigating complex datasets with fuzzy matching, or creating custom menus and
workflows, fzf provides the building blocks to turn shell scripts into rich
terminal applications.

The install script will add lines to your shell configuration file to modify
`$PATH` and set up shell integration.

You can download the official fzf binaries from the releases page.

Add the following line to your shell configuration file.

Without STDIN pipe, fzf will traverse the file system under the current
directory to get the list of files.

With `--height HEIGHT[%]`, fzf will start below the cursor with the given height.

By prepending `~` to the height, you're setting the maximum height.

With `--popup` option, fzf will start in a popup window
(requires tmux 3.3+ or Zellij 0.44+).

The user interface of fzf is fully customizable with a large number of
configuration options. For a quick setup, you can start with one of the style
presets -- `default`, `full`, or `minimal` -- using the `--style` option.

See the man page (`fzf --man` or `man fzf`) for the full list of options.

If you need to post-process the output from fzf, define
`_fzf_complete_COMMAND_post` as follows.

To be fair, running `fzf --print0 | xargs -0 -o vim` instead of `vim "$(fzf)"`
resolves all of the issues mentioned. Nonetheless, `become(...)` still offers
additional benefits in different scenarios.

The following example uses fzf as the selector interface for ripgrep. We bound
`reload` action to `change` event, so every time you type on fzf, the ripgrep
process will restart with the updated query string denoted by the placeholder
expression `{q}`. Also, note that we used `--disabled` option so that fzf
doesn't perform any secondary filtering.

If ripgrep doesn't find any matches, it will exit with a non-zero exit status,
and fzf will warn you about it. To suppress the warning message, we added
`|| true` to the command, so that it always exits with 0.

When the `--preview` option is set, fzf automatically starts an external process
with the current line as the argument and shows the result in the split window.
Your `$SHELL` is used to execute the command with `$SHELL -c COMMAND`.
The window can be scrolled using the mouse or custom key bindings.

See the man page (`man fzf`) for the full list of options.

I would like to thank all the sponsors of this project who make it possible for me to continue to improve fzf.

Create a NodePort service with the specified name.

ObjectMeta is metadata that all persisted resources must have, which includes all objects users must create.

Flask offers suggestions, but doesn't enforce any dependencies or
project layout. It is up to the developer to choose the tools and
libraries they want to use. There are many extensions provided by the
community that make adding new functionality easy.

These components provide the runtime components necessary for building
an asynchronous application.

Please note upstream CPython 3.9 and 3.10 still bundle pip releases whose certificate bundles trust the affected eMudhra/emSign root. We recommend migrating off of those versions and will no longer be releasing Docker images with uv installed for EOL Python versions.

The default stable Python version has changed from 3.14 to 3.15. This affects Python downloads when no version is requested or pinned, e.g., when running `uv python install`.

Previously, uv ignored `--require-hashes` in constraints files included with `-c` from a requirements file. Now, uv honors the directive and requires hashes for all requirements in the installation. Installs that previously succeeded can now fail if a requirement is missing a hash.

Previously, ARM64 builds of uv preferred emulated `x86_64` Python installations because native wheel support was limited. Now, uv prefers native ARM64 (`aarch64`) interpreters across Python versions.

When a native interpreter is unavailable, uv continues to fall back to `x86_64`, then 32-bit `x86`.

You cannot opt out of this behavior. Existing virtual environments are not modified automatically. Recreate an environment with Python 3.10 or later to remove the patch.

Previously, uv split values passed to `--constraint`, `--override`, `--exclude`, and `--build-constraint` on spaces, even when quoted. Now, each value is treated as a single path, allowing file paths containing spaces.

You cannot opt out of this behavior. Repeat the option to provide multiple files. For example, replace `-c "a.txt b.txt"` with `-c a.txt -c b.txt`.

Previously, uv used `astral-tokio-tar` to extract tar archives, build source distributions with `uv_build`, and read their metadata for `uv publish`. Now, uv uses `tar-codec`, which applies stricter validation when reading archives.

You can opt out of this behavior by setting `UV_LEGACY_TAR_BACKEND=1`.

Previously, `uv build --clear` could delete a project or input source distribution when the
output directory contained the source. Now, uv rejects these output directories, including
equivalent paths reached through symlinks, before clearing any build output.

Select an output directory that does not contain any build sources, or omit `--clear`.

All notable changes to this project will be documented in this file.

The 0.28 release includes a limited set of deprecations...

We are working towards a simplified SSL configuration API.

The 0.21.0 release integrates against a newly redesigned `httpcore` backend.

Both packages ought to automatically update to the required versions, but if you are
seeing any issues, you should ensure that you have `httpx==0.21.*` and `httpcore==0.14.*` installed.

The 0.20.0 release adds an integrated command-line client, and also includes some
design changes. The most notable of these is that redirect responses are no longer
automatically followed, unless specifically requested.

This design decision prioritises a more explicit approach to redirects, in order
to avoid code that unintentionally issues multiple requests as a result of
misconfigured URLs.

For example, previously a client configured to send requests to `http://api.github.com/`
would end up sending every API request twice, as each request would be redirected to `https://api.github.com/`.

If you do want auto-redirect behaviour, you can enable this either by configuring
the client instance with `Client(follow_redirects=True)`, or on a per-request
basis, with `.get(..., follow_redirects=True)`.

The other major design change is an update to the Transport API, which is the low-level
interface against which requests are sent. Previously this interface used only primitive
datastructures, like so...

The 0.18.x release series formalises our low-level Transport API, introducing the base classes `httpx.BaseTransport` and `httpx.AsyncBaseTransport`.

Pull request #1522 includes a checklist of differences from the previous `httpcore` transport API, for developers implementing custom transports.

The following API changes have been issuing deprecation warnings since 0.17.0 onwards, and are now fully deprecated...

The 0.14 release includes a range of improvements to the public API, intended on preparing for our upcoming 1.0 release.

When upgrading you should be aware of the following public API changes. Note that deprecated usages will currently continue to function, but will issue warnings.

One notable aspect of the 0.14.0 release is that it tightens up the public API for `httpx`, by ensuring that several internal attributes and methods have now become strictly private.

The following previously had nominally public names on the client, but were all undocumented and intended solely for internal usage. They are all now replaced with underscored names, and should not be relied on or accessed.

These changes should not affect users who have been working from the `httpx` documentation.

Some areas of API which were already on the deprecation path, and were raising warnings or errors in 0.13.x have now been escalated to being fully removed.

Pydantic V2 introduces a number of changes to the API, including some breaking changes.

This page provides a guide highlighting the most
important changes to help you migrate your code from Pydantic V1 to Pydantic V2.

We have created a tool to help you migrate your code. This tool is still in beta, but we hope it will help you to
migrate your code more quickly.

Pydantic V1 is still available when you need it, though we recommend migrating to
Pydantic V2 for its improvements and new features.

The Pydantic V2 package also continues to provide access to the Pydantic V1 API
by importing through `pydantic.v1`.

The following sections provide details on the most important changes in Pydantic V2.

Various method names have been changed; all non-deprecated `BaseModel` methods now have names matching either the
format `model_.*` or `__.*pydantic.*__`. Where possible, we have retained the deprecated methods with their old names
to help ease migration, but calling them will emit `DeprecationWarning`s.

All notable changes to this project will be documented in this file.

Omitted `bool` and `bool | None` form fields now use their declared defaults or produce missing-field errors, consistent with other fields and Pydantic behavior. FastAPI no longer infers `False` for omitted booleans. This applies to individual form parameters, form models, and unions of form models.

Omitted boolean model fields are excluded from `model_dump(exclude_unset=True)`, and their defaults no longer count as submitted fields during union selection. Default factories and `validate_default` follow the usual rules.

Query, header, cookie, and form parameter models now apply their `extra` configuration to all submitted fields in the same parameter location, consistent with Pydantic behavior. This includes fields declared in other models, individual parameters, or dependencies, which were previously excluded.

With `extra="forbid"`, fields outside the model are rejected even if another parameter declares them. With `extra="allow"`, those fields are included as model extras. With the default `extra="ignore"`, they are ignored.

The deprecated `**extra` keyword arguments have been removed from `Path`, `Query`, `Header`, `Cookie`, `Body`, `Form`, and `File`. Unknown keyword arguments now raise `TypeError`. For additional JSON Schema properties, replace, for example, `Query(custom="value")` with `Query(json_schema_extra={"custom": "value"})`.

Unions of form models now apply the same browser form handling as individual models. Empty scalar fields use defaults or produce missing-field errors, empty list items are preserved, omitted boolean fields become `False`, and unselected files are treated as missing. JSON request bodies are unchanged.

Validation errors for header parameter models now report the HTTP header name, such as `int-val`, instead of the Python field name, `int_val`. Explicit aliases, `Header(convert_underscores=False)`, and Pydantic models with `loc_by_alias=False` keep their existing behavior.

JSON request bodies now distinguish an explicit `null` from an omitted value. For individual `Body` parameters, `null` is validated as `None`: nullable types accept it, and non-nullable types reject it even when a default is defined. Defaults and default factories apply only when the value is omitted. This applies both to parameters inside JSON objects and to parameters representing the entire body. Clients that used `null` to request a default should omit the value instead.

When a JSON object is expected because of `Body(embed=True)` or multiple body parameters, sending `null` as the entire body now returns a validation error.

When no file is selected, browsers submit a file part with an empty filename and empty content. FastAPI now treats this as missing: required file fields return a validation error, and fields with defaults or default factories use them. Unselected inputs are ignored in lists of files. Named zero-byte files are still accepted.

Browsers submit empty text inputs as empty strings (`""`). FastAPI now treats empty scalar form values as missing in form models, matching individual `Form` parameters. Required fields return a validation error. Fields with defaults or default factories use them and remain unset in `model_dump(exclude_unset=True)`.

Empty strings inside lists are preserved. Omitted boolean form fields still become `False`. JSON requests and direct Pydantic validation are unchanged.

Defaults for omitted `Query`, `Header`, `Cookie`, and `Form` parameters and model fields are no longer treated as user input. Defaults are not validated unless `validate_default=True` is enabled. If you rely on validators running for omitted values, enable default validation with `Field(validate_default=True)` for model fields or, for example, `Query(validate_default=True)` for individual parameters. `Body(validate_default=True)` is now honored too.

For parameter models, applying a default no longer marks a field as explicitly set, so `model_dump(exclude_unset=True)` excludes it. Default factories run once, and validation errors no longer include injected defaults in `input`.

Omitted boolean form fields still become `False` and count as explicitly set.

Omitted `bool` and `bool | None` form fields are now interpreted as `False`, matching unchecked HTML checkboxes. This applies to individual `Form` parameters and form models, including required fields and fields with defaults of `True` or `None`. The inferred value is considered explicitly set, including in `model_dump(exclude_unset=True)`.

The `root_path` parameter in `FastAPI()` is deprecated. Configure the server with `fastapi run --root-path /api/v1` instead.

Automatic OpenTelemetry exporter setup is now opt-in. If you rely on FastAPI to configure exporters from `OTEL_*` environment variables, set `FASTAPI_OTEL_AUTO_CONFIGURE=true` or use `FastAPI(telemetry={"auto_configure": True})` to retain the previous behavior.

Applications that configure their own providers and exporters require no changes. FastAPI Cloud enables automatic setup for you.

Before this, `router.include_router(other_router)` would take each path operation from `other_router` and "clone" it, or recreate it from scratch.

This would mean that in the end there was only one top level router, part of the app.

The way it is structured here is that there are a few additional classes to handle intermediate metadata for router and route inclusion. That way the information of "router X includes Y and Y includes Z" is stored somewhere, without affecting (recreating / cloning) the final route.

Now `router.routes` is no longer a plain list of `APIRoute` objects, it can contain these intermediate objects that can contain additional routers, forming a tree.

Any logic that depended on iterating on the `router.routes` directly would be affected, that logic cannot expect to be able to extract data from a plain list of routes, as it's no longer a plain list but a tree.

Additionally, any logic that iterated on `router.routes` to modify them would now also see these new objects, and would not see all the routes in the app.

This is not documented yet, so it's not officially supported yet and could change in the future.

But, as `APIRoute` and `APIRouter` instances are now preserved, they could be customized.

Still, for now, consider this very experimental and potentially changing and breaking in the future.

This checklist aims to provide basic guidelines on securing applications
running in Kubernetes from a developer's perspective.
This list is not meant to be exhaustive and is intended to evolve over time.

The following checklist provides base security hardening recommendations that
would apply to most applications deploying to Kubernetes.

For sensitive workloads, consider providing a recommended ValidatingAdmissionPolicy
that further restricts the permitted write actions.

Make sure that your cluster provides and enforces NetworkPolicy.
If you are writing an application that users will deploy to different clusters,
consider whether you can assume that NetworkPolicy is available and enforced.

This section of this guide covers some advanced security hardening points
which might be valuable based on different Kubernetes environment setup.

Some containers may require a different isolation level from what is provided by
the default runtime of the cluster. `runtimeClassName` can be used in a podspec
to define a different runtime class.

Read on for an overview of how Kubernetes is designed to help you deploy a
secure cloud native platform.

When you deploy Kubernetes, you also set the foundation for your
applications' runtime environment: a Kubernetes cluster (or
multiple clusters).
That infrastructure must provide the security guarantees that higher
layers expect.

The Kubernetes API is what makes your cluster work. Protecting this API is key
to providing effective cluster security.

The network plugin you choose and the way you integrate it can have a
strong impact on the security of information in transit.

Kubernetes lets you extend your cluster with extra tooling. You can set up third
party solutions to help you monitor or troubleshoot your applications and the
clusters they are running. You also get some basic observability features built
in to Kubernetes itself. Your code running in containers can generate logs,
publish metrics, or provide other observability data; at deploy time, you need to
make sure your cluster provides an appropriate level of protection there.

If you set up a metrics dashboard or something similar, review the chain of components
that populate data into that dashboard, as well as the dashboard itself. Make sure
that the whole chain is designed with enough resilience and integrity protection
that you can rely on it even during an incident where your cluster might be degraded.

Where appropriate, deploy security measures below the Kubernetes layer,
such as cryptographically measured boot or authenticated distribution
of time (which helps ensure the fidelity of logs and audit records).

For a high-assurance environment, deploy cryptographic protections to ensure that
logs are both tamper-proof and confidential.

By default, the Kubernetes API server listens on port 6443 on the first non-localhost
network interface, protected by TLS. In a typical production Kubernetes cluster, the
API serves on port 443. The port can be changed with the `--secure-port`, and the
listening IP address with the `--bind-address` flag.

The API server presents a certificate. This certificate may be signed using
a private certificate authority (CA), or based on a public key infrastructure linked
to a generally recognized CA. The certificate and corresponding private key can be set
by using the `--tls-cert-file` and `--tls-private-key-file` flags.

If your cluster uses a private certificate authority, you need a copy of that CA
certificate configured into your `~/.kube/config` on the client, so that you can
trust the connection and be confident it was not intercepted.

Your client can present a TLS client certificate at this stage.

The input to the authentication step is the entire HTTP request; however, it typically
examines the headers and/or client certificate.

Authentication modules include client certificates, password, and plain tokens,
bootstrap tokens, and JSON Web Tokens (used for service accounts).

Multiple authentication modules can be specified, in which case each one is tried in sequence,
until one of them succeeds.

If the request cannot be authenticated, it is rejected with HTTP status code 401.
Otherwise, the user is authenticated as a specific `username`, and the user name
is available to subsequent steps to use in their decisions.  Some authenticators
also provide the group memberships of the user, while other authenticators
do not.

While Kubernetes uses usernames for access control decisions and in request logging,
it does not have a `User` object nor does it store usernames or other information about
users in its API.

A request must include the username of the requester, the requested action, and
the object affected by the action. The request is authorized if an existing policy
declares that the user has permissions to complete the requested action.

If Bob makes a request to write (`create` or `update`) to the objects in the
`projectCaribou` namespace, his authorization is denied. If Bob makes a request
to read (`get`) objects in a different namespace such as `projectFish`, then his authorization is denied.

Kubernetes authorization requires that you use common REST attributes to interact
with existing organization-wide or cloud-provider-wide access control systems.
It is important to use REST formatting because these control systems might
interact with other APIs besides the Kubernetes API.

Kubernetes supports multiple authorization modules, such as ABAC mode, RBAC Mode,
and Webhook mode. When an administrator creates a cluster, they configure the
authorization modules that should be used in the API server. If more than one
authorization modules are configured, Kubernetes checks each module, and if
any module authorizes the request, then the request can proceed. If all of
the modules deny the request, then the request is denied (HTTP status code 403).

Admission Control modules are software modules that can modify or reject requests.
In addition to the attributes available to Authorization modules, Admission
Control modules can access the contents of the object that is being created or modified.

Admission controllers act on requests that create, modify, delete, or connect to (proxy) an object.
Admission controllers do not act on requests that merely read objects.
When multiple admission controllers are configured, they are called in order.

Unlike Authentication and Authorization modules, if any admission controller module
rejects, then the request is immediately rejected.

In addition to rejecting objects, admission controllers can also set complex defaults for
fields.

Kubernetes auditing provides a security-relevant, chronological set of records documenting the sequence of actions in a cluster.
The cluster audits the activities generated by users, by applications that use the Kubernetes API, and by the control plane itself.

This page provides an overview of available configuration options and best practices for cluster
multi-tenancy.

While Kubernetes does not have first-class concepts of end users or tenants, it provides several
features to help manage different tenancy requirements. These are discussed below.

The first step to determining how to share your cluster is understanding your use case, so you can
evaluate the patterns and tools available. In general, multi-tenancy in Kubernetes clusters falls
into two broad categories, though many variations and hybrids are also possible.

A common form of multi-tenancy is to share a cluster between multiple teams within an
organization, each of whom may operate one or more workloads. These workloads frequently need to
communicate with each other, and with other workloads located on the same or different clusters.

In this scenario, members of the teams often have direct access to Kubernetes resources via tools
such as `kubectl`, or indirect access through GitOps controllers or other types of release
automation tools. There is often some level of trust between members of different teams, but
Kubernetes policies such as RBAC, quotas, and network policies are essential to safely and fairly
share clusters.

The other major form of multi-tenancy frequently involves a Software-as-a-Service (SaaS) vendor
running multiple instances of a workload for customers. This business model is so strongly
associated with this deployment style that many people call it "SaaS tenancy." However, a better
term might be "multi-customer tenancy," since SaaS vendors may also use other deployment models,
and this deployment model can also be used outside of SaaS.

In this scenario, the customers do not have access to the cluster; Kubernetes is invisible from
their perspective and is only used by the vendor to manage the workloads. Cost optimization is
frequently a critical concern, and Kubernetes policies are used to ensure that the workloads are
strongly isolated from each other.

When discussing multi-tenancy in Kubernetes, there is no single definition for a "tenant".
Rather, the definition of a tenant will vary depending on whether multi-team or multi-customer
tenancy is being discussed.

In multi-team usage, a tenant is typically a team, where each team typically deploys a small
number of workloads that scales with the complexity of the service. However, the definition of
"team" may itself be fuzzy, as teams may be organized into higher-level divisions or subdivided
into smaller teams.

By contrast, if each team deploys dedicated workloads for each new client, they are using a
multi-customer model of tenancy. In this case, a "tenant" is simply a group of users who share a
single workload. This may be as large as an entire company, or as small as a single team at that
company.

In many cases, the same organization may use both definitions of "tenants" in different contexts.
For example, a platform team may offer shared services such as security tools and databases to
multiple internal “customers” and a SaaS vendor may also have multiple teams sharing a development
cluster. Finally, hybrid architectures are also possible, such as a SaaS provider using a
combination of per-customer workloads for sensitive data, combined with multi-tenant shared
services.

There are several ways to design and build multi-tenant solutions with Kubernetes. Each of these
methods comes with its own set of tradeoffs that impact the isolation level, implementation
effort, operational complexity, and cost of service.

A Kubernetes cluster consists of a control plane which runs Kubernetes software, and a data plane
consisting of worker nodes where tenant workloads are executed as pods. Tenant isolation can be
applied in both the control plane and the data plane based on organizational requirements.

The level of isolation offered is sometimes described using terms like “hard” multi-tenancy, which
implies strong isolation, and “soft” multi-tenancy, which implies weaker isolation. In particular,
"hard" multi-tenancy is often used to describe cases where the tenants do not trust each other,
often from security and resource sharing perspectives (e.g. guarding against attacks such as data
exfiltration or DoS). Since data planes typically have much larger attack surfaces, "hard"
multi-tenancy often requires extra attention to isolating the data-plane, though control plane
isolation  also remains critical.

However, the terms "hard" and "soft" can often be confusing, as there is no single definition that
will apply to all users. Rather, "hardness" or "softness" is better understood as a broad
spectrum, with many different techniques that can be used to maintain different types of isolation
in your clusters, based on your requirements.

The remainder of this page focuses on isolation techniques used for shared Kubernetes clusters.
However, even if you are considering dedicated clusters, it may be valuable to review these
recommendations, as it will give you the flexibility to shift to shared clusters in the future if
your needs or capabilities change.

Control plane isolation ensures that different tenants cannot access or affect each others'
Kubernetes API resources.

In a multi-tenant environment, a Namespace helps segment a tenant's workload into a logical and
distinct management unit. In fact, a common practice is to isolate every workload in its own
namespace, even if multiple workloads are operated by the same tenant. This ensures that each
workload has its own identity and can be configured with an appropriate security policy.

The namespace isolation model requires configuration of several other Kubernetes resources,
networking plugins, and adherence to security best practices to properly isolate tenant workloads.
These considerations are discussed below.

In a multi-team environment, RBAC must be used to restrict tenants' access to the appropriate
namespaces, and ensure that cluster-wide resources can only be accessed or modified by privileged
users such as cluster administrators.

If a policy ends up granting a user more permissions than they need, this is likely a signal that
the namespace containing the affected resources should be refactored into finer-grained
namespaces. Namespace management tools may simplify the management of these finer-grained
namespaces by applying common RBAC policies to different namespaces, while still allowing
fine-grained policies where necessary.

Resource quotas are namespaced objects. By mapping tenants to namespaces, cluster admins can use
quotas to ensure that a tenant cannot monopolize a cluster's resources or overwhelm its control
plane. Namespace management tools simplify the administration of quotas. In addition, while
Kubernetes quotas only apply within a single namespace, some namespace management tools allow
groups of namespaces to share quotas, giving administrators far more flexibility with less effort
than built-in quotas.

Quotas prevent a single tenant from consuming greater than their allocated share of resources
hence minimizing the “noisy neighbor” issue, where one tenant negatively impacts the performance
of other tenants' workloads.

When you apply a quota to namespace, Kubernetes requires you to also specify resource requests and
limits for each container. Limits are the upper bound for the amount of resources that a container
can consume. Containers that attempt to consume resources that exceed the configured limits will
either be throttled or killed, based on the resource type. When resource requests are set lower
than limits, each container is guaranteed the requested amount but there may still be some
potential for impact across workloads.

Quotas cannot protect against all kinds of resource sharing, such as network traffic.
Node isolation (described below) may be a better solution for this problem.

Data plane isolation ensures that pods and workloads for different tenants are sufficiently
isolated.

By default, all pods in a Kubernetes cluster are allowed to communicate with each other, and all
network traffic is unencrypted. This can lead to security vulnerabilities where traffic is
accidentally or maliciously sent to an unintended destination, or is intercepted by a workload on
a compromised node.

Namespace management tools may simplify the creation of default or common network policies.
In addition, some of these tools allow you to enforce a consistent set of namespace labels across
your cluster, ensuring that they are a trusted basis for your policies.

More advanced network isolation may be provided by service meshes, which provide OSI Layer 7
policies based on workload identity, in addition to namespaces. These higher-level policies can
make it easier to manage namespace-based multi-tenancy, especially when multiple namespaces are
dedicated to a single tenant. They frequently also offer encryption using mutual TLS, protecting
your data even in the presence of a compromised node, and work across dedicated or virtual clusters.
However, they can be significantly more complex to manage and may not be appropriate for all users.

Kubernetes pods are composed of one or more containers that execute on worker nodes.
Containers utilize OS-level virtualization and hence offer a weaker isolation boundary than
virtual machines that utilize hardware-based virtualization.

In a shared environment, unpatched vulnerabilities in the application and system layers can be
exploited by attackers for container breakouts and remote code execution that allow access to host
resources. In some applications, like a Content Management System (CMS), customers may be allowed
the ability to upload and execute untrusted scripts or code. In either case, mechanisms to further
isolate and protect workloads using strong isolation are desirable.

Virtual machines and userspace kernels are two popular approaches to sandboxing.

Node isolation is another technique that you can use to isolate tenant workloads from each other.
With node isolation, a set of nodes is dedicated to running pods from a particular tenant and
co-mingling of tenant pods is prohibited. This configuration reduces the noisy tenant issue, as
all pods running on a node will belong to a single tenant. The risk of information disclosure is
slightly lower with node isolation because an attacker that manages to escape from a container
will only have access to the containers and volumes mounted to that node.

Although workloads from different tenants are running on different nodes, it is important to be
aware that the kubelet and (unless using virtual control planes) the API service are still shared
services. A skilled attacker could use the permissions assigned to the kubelet or other pods
running on the node to move laterally within the cluster and gain access to tenant workloads
running on other nodes. If this is a major concern, consider implementing compensating controls
such as seccomp, AppArmor or SELinux or explore using sandboxed containers or creating separate
clusters for each tenant.

Node isolation is a little easier to reason about from a billing standpoint than sandboxing
containers since you can charge back per node rather than per pod. It also has fewer compatibility
and performance issues and may be easier to implement than sandboxing containers.
For example, nodes for each tenant can be configured with taints so that only pods with the
corresponding toleration can run on them. A mutating webhook could then be used to automatically
add tolerations and node affinities to pods deployed into tenant namespaces so that they run on a
specific set of nodes designated for that tenant.

This section discusses other Kubernetes constructs and patterns that are relevant for multi-tenancy.

Using API priority and fairness will not be very common in SaaS environments unless you are
allowing customers to run applications that interface with the Kubernetes API, for example,
a controller.

When you’re running a SaaS application, you may want the ability to offer different
Quality-of-Service (QoS) tiers of service to different tenants. For example, you may have freemium
service that comes with fewer performance guarantees and features and a for-fee service tier with
specific performance guarantees. Fortunately, there are several Kubernetes constructs that can
help you accomplish this within a shared cluster, including network QoS, storage classes, and pod
priority and preemption. The idea with each of these is to provide tenants with the quality of
service that they paid for. Let’s start by looking at networking QoS.

For storage QoS, you will likely want to create different storage classes or profiles with
different performance characteristics. Each storage profile can be associated with a different
tier of service that is optimized for different workloads such IO, redundancy, or throughput.
Additional logic might be necessary to allow the tenant to associate the appropriate storage
profile with their workload.

Kubernetes clusters include a Domain Name System (DNS) service to provide translations from names
to IP addresses, for all Services and Pods. By default, the Kubernetes DNS service allows lookups
across all namespaces in the cluster.

There are two primary ways to share a Kubernetes cluster for multi-tenancy: using Namespaces
(that is, a Namespace per tenant) or by virtualizing the control plane (that is, virtual control
plane per tenant).

In both cases, data plane isolation, and management of additional considerations such as API
Priority and Fairness, is also recommended.

Namespace isolation is well-supported by Kubernetes, has a negligible resource cost, and provides
mechanisms to allow tenants to interact appropriately, such as by allowing service-to-service
communication. However, it can be difficult to configure, and doesn't apply to Kubernetes
resources that can't be namespaced, such as Custom Resource Definitions, Storage Classes, and Webhooks.

Control plane virtualization allows for isolation of non-namespaced resources at the cost of
somewhat higher resource usage and more difficult cross-tenant sharing. It is a good option when
namespace isolation is insufficient but dedicated clusters are undesirable, due to the high cost
of maintaining them (especially on-prem) or due to their higher overhead and lack of resource
sharing. However, even within a virtualized control plane, you will likely see benefits by using
namespaces as well.

The two options are discussed in more detail in the following sections.

As previously mentioned, you should consider isolating each workload in its own namespace, even if
you are using dedicated clusters or virtualized control planes. This ensures that each workload
only has access to its own resources, such as ConfigMaps and Secrets, and allows you to tailor
dedicated security policies for each workload. In addition, it is a best practice to give each
namespace names that are unique across your entire fleet (that is, even if they are in separate
clusters), as this gives you the flexibility to switch between dedicated and shared clusters in
the future, or to use multi-cluster tooling such as service meshes.

Conversely, there are also advantages to assigning namespaces at the tenant level, not just the
workload level, since there are often policies that apply to all workloads owned by a single
tenant. However, this raises its own problems. Firstly, this makes it difficult or impossible to
customize policies to individual workloads, and secondly, it may be challenging to come up with a
single level of "tenancy" that should be given a namespace. For example, an organization may have
divisions, teams, and subteams - which should be assigned a namespace?

One possible approach is to organize your namespaces into hierarchies, and share certain policies and
resources between them. This could include managing namespace labels, namespace lifecycles,
delegated access, and shared resource quotas across related namespaces. These capabilities can
be useful in both multi-team and multi-customer scenarios.

A virtual control plane typically consists of the Kubernetes API server, the controller manager,
and the etcd data store. It interacts with the super cluster via a metadata synchronization
controller which coordinates changes across tenant control planes and the control plane of the
super-cluster.

By using per-tenant dedicated control planes, most of the isolation problems due to sharing one
API server among all tenants are solved. Examples include noisy neighbors in the control plane,
uncontrollable blast radius of policy misconfigurations, and conflicts between cluster scope
objects such as webhooks and CRDs.  Hence, the virtual control plane model is particularly
suitable for cases where each tenant requires access to a Kubernetes API server and expects the
full cluster manageability.

The improved isolation comes at the  cost of running and maintaining an individual virtual control
plane per tenant. In addition, per-tenant control planes do not solve isolation problems in the
data plane, such as node-level noisy neighbors or security threats. These must still be addressed
separately.

A namespace can configure any or all modes, or even set a different level for different modes.

The Privileged policy is defined by an absence of restrictions. If you define a Pod where the Privileged
security policy applies, the Pod you define is able to bypass typical container isolation mechanisms.
For example, you can define a Pod that has access to the node's host network.

Decoupling policy definition from policy instantiation allows for a common understanding and
consistent language of policies across clusters, independent of the underlying enforcement
mechanism.

As mechanisms mature, they will be defined below on a per-policy basis. The methods of enforcement
of individual policies are not defined here.

The three profiles defined here have a clear linear progression from most secure (Restricted) to least
secure (Privileged), and cover a broad set of workloads. Privileges required above the Baseline
policy are typically very application specific, so we do not offer a standard profile in this
niche. This is not to say that the privileged profile should always be used in this case, but that
policies in this space need to be defined on a case-by-case basis.

SIG Auth may reconsider this position in the future, should a clear need for other profiles arise.

There is currently no API standard that controls whether a Pod is considered sandboxed or
not. Sandbox Pods may be identified by the use of a sandboxed runtime (such as gVisor or Kata
Containers), but there is no standard definition of what a sandboxed runtime is.

The protections necessary for sandboxed workloads can differ from others. For example, the need to
restrict privileged permissions is lessened when the workload is isolated from the underlying
kernel. This allows for workloads requiring heightened permissions to still be isolated.

Additionally, the protection of sandboxed workloads is highly dependent on the method of
sandboxing. As such, no single recommended profile is recommended for all sandboxed workloads.

Some steps also involve installing `curl`, which can be installed via your
operating system's software sources.

A subset of the kubelet's configuration parameters may be
set via an on-disk config file, as a substitute for command-line flags.

Providing parameters via a config file is the recommended approach because
it simplifies node deployment and configuration management.

The configuration file must be a JSON or YAML representation of the parameters
in this struct. Make sure the kubelet has read permissions on the file.

The `imagefs` is an optional filesystem that container runtimes use to store container
images and container writable layers.

Start the kubelet with the `--config` flag set to the path of the kubelet's config file.
The kubelet will then load its config from this file.

Note that command line flags which target the same value as a config file will override that value.
This helps ensure backwards compatibility with the command-line API.

Note that relative file paths in the kubelet config file are resolved relative to the
location of the kubelet config file, whereas relative paths in command line flags are resolved
relative to the kubelet's current working directory.

Note that some default values differ between command-line flags and the kubelet config file.
If `--config` is provided and the values are not specified via the command line, the
defaults for the `KubeletConfiguration` version apply.
In the above example, this version is `kubelet.config.k8s.io/v1beta1`.

For Kubernetes v1.28 to v1.29, you can only specify `--config-dir` if you also set
the environment variable `KUBELET_CONFIG_DROPIN_DIR_ALPHA` for the kubelet process (the value
of that variable does not matter).

These files may contain partial configurations but should not be invalid and must include type metadata, specifically `apiVersion` and `kind`. 
Validation is only performed on the final resulting configuration structure stored internally in the kubelet. 
This offers flexibility in managing and merging kubelet configurations from different sources while preventing undesirable configurations. 
However, it is important to note that behavior varies based on the data type of the configuration fields.

Unlike kind and minikube, Usernetes supports composing a cluster using multiple Docker/Podman/nerdctl nodes using VXLAN.

Sysbox supports running Kubernetes inside unprivileged containers without
requiring Cgroup v2 and without the `KubeletInUserNamespace` feature gate. It
does this by exposing specially crafted `/proc` and `/sys` filesystems inside
the container plus several other advanced OS virtualization techniques.

This section provides hints for running Kubernetes in a user namespace manually.

If you are trying to run Kubernetes in a user-namespaced container such as
Rootless Docker/Podman or LXC/LXD, you are all set, and you can go to the next subsection.

Otherwise you have to create a user namespace by yourself, by calling `unshare(2)` with `CLONE_NEWUSER`.

After unsharing the user namespace, you will also have to unshare other namespaces such as mount namespace.

In addition to the user namespace, you also need to have a writable cgroup tree with cgroup v2.

If you are trying to run Kubernetes in Rootless Docker/Podman or LXC/LXD on a systemd-based host, you are all set.

Otherwise you have to create a systemd unit with `Delegate=yes` property to delegate a cgroup tree with writable permission.

The network namespaces of the Pods can be configured with regular CNI plugins.
For multi-node networking, Flannel (VXLAN, 8472/UDP) is known to work.

The kubelet relies on a container runtime. You should deploy a container runtime such as
containerd or CRI-O and ensure that it is running within the user namespace before the kubelet starts.

Running CRI plugin of containerd in a user namespace is supported since containerd 1.4.

Running containerd within a user namespace requires the following configurations.

When the `KubeletInUserNamespace` feature gate is enabled, the kubelet ignores errors
that may happen during setting the following sysctl values on the node.

Within a user namespace, the kubelet also ignores any error raised from trying to open `/dev/kmsg`.
This feature gate also allows kube-proxy to ignore an error during setting `RLIMIT_NOFILE`.

The `KubeletInUserNamespace` feature gate was introduced in Kubernetes v1.22 with "alpha" status.
