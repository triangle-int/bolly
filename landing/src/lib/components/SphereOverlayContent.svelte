<script lang="ts">
	import { onMount } from 'svelte';
	import { T, useTask, useThrelte } from '@threlte/core';
	import {
		MeshPhysicalNodeMaterial,
		MeshBasicNodeMaterial,
		MeshBasicMaterial,
		CanvasTexture,
		Color,
		IcosahedronGeometry,
		SphereGeometry,
		PlaneGeometry,
		FrontSide,
		BackSide,
		SRGBColorSpace,
		PMREMGenerator,
	} from 'three/webgpu';
	import type { WebGPURenderer } from 'three/webgpu';
	import {
		Fn, uniform, float, vec3, vec4,
		uv, positionLocal, time, screenUV,
		sin, cos, pow,
	} from 'three/tsl';

	const { scene, renderer, camera, invalidate } = useThrelte();

	// ── Background plane — sized to fill camera view ──
	const CAM_Z = 5;
	const PLANE_Z = -1;
	const FOV = 45;
	const dist = CAM_Z - PLANE_Z;
	const visH = 2 * dist * Math.tan((FOV / 2) * Math.PI / 180);
	const aspect = typeof window !== 'undefined' ? window.innerWidth / window.innerHeight : 16 / 9;
	const visW = visH * aspect;

	const bgGeo = new PlaneGeometry(visW, visH);
	const bgMat = new MeshBasicMaterial({ color: 0xffffff });

	// Dark scene background
	scene.background = new Color(0x000206);

	// ── Skybox for envmap ──
	const skyGeo = new SphereGeometry(30, 64, 32);
	const skyMat = new MeshBasicNodeMaterial({ side: BackSide });
	const skyShader = Fn(() => {
		const st = uv();
		const t = time.mul(0.012);
		const E = float(2.718);
		const c = vec3(0.005, 0.008, 0.025).toVar();
		c.addAssign(vec3(0.003, 0.005, 0.015).mul(st.y));
		const TAU = float(6.2832);
		const sx = st.x.mul(TAU);
		const band = (baseY: number, drift: number, phase: number, w: number, col: [number, number, number], bright: number) => {
			const wy = float(baseY).add(sin(sx.mul(0.8).add(t.mul(drift)).add(phase)).mul(0.06));
			const d = st.y.sub(wy);
			c.addAssign(vec3(col[0], col[1], col[2]).mul(pow(E, d.mul(d).negate().div(float(w * w * 2)))).mul(bright));
		};
		band(0.6, 0.4, 0, 0.14, [0.04, 0.12, 0.18], 0.15);
		band(0.4, 0.3, 2, 0.16, [0.06, 0.05, 0.15], 0.10);
		band(0.25, 0.5, 4, 0.10, [0.10, 0.03, 0.12], 0.08);
		return vec4(c, float(1));
	});
	skyMat.colorNode = skyShader();

	// ── Glass material ──
	const uSpeed = uniform(0.8);
	const uIntensity = uniform(0.10);
	const uBreathe = uniform(1.0);

	const displacedPos = Fn(() => {
		const pos = positionLocal.toVar();
		const t = time;
		const noise = sin(pos.x.mul(2.1).add(t.mul(uSpeed).mul(0.7)))
			.mul(cos(pos.y.mul(1.8).add(t.mul(uSpeed).mul(0.5))))
			.mul(sin(pos.z.mul(2.5).add(t.mul(uSpeed).mul(0.9))));
		return pos.mul(uBreathe.add(noise.mul(uIntensity)));
	});

	const glassMat = new MeshPhysicalNodeMaterial();
	glassMat.positionNode = displacedPos();
	glassMat.color = new Color(0xffffff);
	glassMat.transmission = 1.0;
	glassMat.ior = 1.5;
	glassMat.thickness = 2.5;
	glassMat.roughness = 0.0;
	glassMat.metalness = 0.0;
	glassMat.dispersion = 0.15;
	glassMat.attenuationColor = new Color(0xffffff);
	glassMat.attenuationDistance = Infinity;
	glassMat.specularIntensity = 0.2;
	glassMat.specularColor = new Color(0xffffff);
	glassMat.envMapIntensity = 1.5;
	glassMat.clearcoat = 1.0;
	glassMat.clearcoatRoughness = 0.1;
	glassMat.transparent = true;
	glassMat.side = FrontSide;

	const mainGeo = new IcosahedronGeometry(1, 6);
	const smallGeo = new IcosahedronGeometry(0.45, 6);
	const tinyGeo = new IcosahedronGeometry(0.25, 6);

	// ── Envmap ──
	let envDone = false;
	const pmrem = new PMREMGenerator(renderer as unknown as InstanceType<typeof WebGPURenderer>);

	// ── foreignObject → SVG → Canvas → Texture (fast, native browser render) ──
	let captureCanvas: HTMLCanvasElement;
	let captureCtx: CanvasRenderingContext2D;
	let bgTexture: InstanceType<typeof CanvasTexture> | null = null;

	onMount(() => {
		let cleanup: (() => void) | undefined;
		void (async () => {
		await new Promise(r => setTimeout(r, 600));

		const pageEl = document.getElementById('page-capture-source');
		if (!pageEl) return;

		const w = window.innerWidth;
		const h = window.innerHeight;
		const dpr = Math.min(window.devicePixelRatio, 1.5);

		captureCanvas = document.createElement('canvas');
		captureCanvas.width = w * dpr;
		captureCanvas.height = h * dpr;
		captureCtx = captureCanvas.getContext('2d')!;

		// Gather all stylesheets as inline CSS for foreignObject
		let styles = '';
		for (const sheet of document.styleSheets) {
			try {
				for (const rule of sheet.cssRules) {
					styles += rule.cssText + '\n';
				}
			} catch { /* cross-origin stylesheets */ }
		}

		async function capture() {
			try {
				// Force reveals visible
				pageEl!.classList.add('capture-mode');

				// Get the visible portion's HTML
				const clone = pageEl!.cloneNode(true) as HTMLElement;

				// Build SVG with foreignObject
				const svgStr = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}">
					<foreignObject width="100%" height="100%">
						<div xmlns="http://www.w3.org/1999/xhtml" style="
							width: ${w}px;
							height: ${h}px;
							overflow: hidden;
							transform: translateY(${-window.scrollY}px);
						">
							<style>${styles}</style>
							${clone.outerHTML}
						</div>
					</foreignObject>
				</svg>`;

				const blob = new Blob([svgStr], { type: 'image/svg+xml;charset=utf-8' });
				const url = URL.createObjectURL(blob);

				const img = new Image();
				img.width = w;
				img.height = h;

				await new Promise<void>((resolve, reject) => {
					img.onload = () => {
						captureCtx.clearRect(0, 0, captureCanvas.width, captureCanvas.height);
						captureCtx.drawImage(img, 0, 0, captureCanvas.width, captureCanvas.height);
						URL.revokeObjectURL(url);

						if (!bgTexture) {
							bgTexture = new CanvasTexture(captureCanvas);
							bgTexture.colorSpace = SRGBColorSpace;
							bgMat.map = bgTexture;
							bgMat.needsUpdate = true;
							console.log('[SphereContent] foreignObject texture applied');
						} else {
							bgTexture.needsUpdate = true;
						}

						resolve();
					};
					img.onerror = (e) => {
						URL.revokeObjectURL(url);
						reject(e);
					};
				});

				pageEl!.classList.remove('capture-mode');
			} catch (e) {
				pageEl!.classList.remove('capture-mode');
				console.warn('[SphereContent] Capture error:', e);
			}
		}

		// Initial capture
		await capture();

		// Fast re-capture on scroll (~30fps capable)
		let rafId: number;
		let dirty = true;

		function markDirty() { dirty = true; }
		window.addEventListener('scroll', markDirty, { passive: true });

		function loop() {
			if (dirty) {
				dirty = false;
				capture();
			}
			rafId = requestAnimationFrame(loop);
		}
		rafId = requestAnimationFrame(loop);

		cleanup = () => {
			window.removeEventListener('scroll', markDirty);
			cancelAnimationFrame(rafId);
			bgTexture?.dispose();
		};
		})();
		return () => cleanup?.();
	});

	// ── Scroll & mouse ──
	let scrollProgress = 0;
	let smoothScroll = 0;
	let mouseX = 0;
	let mouseY = 0;
	let targetMX = 0;
	let targetMY = 0;

	if (typeof window !== 'undefined') {
		window.addEventListener('scroll', () => {
			const max = document.body.scrollHeight - window.innerHeight;
			scrollProgress = max > 0 ? window.scrollY / max : 0;
		}, { passive: true });
		window.addEventListener('mousemove', (e: MouseEvent) => {
			targetMX = (e.clientX / window.innerWidth - 0.5) * 2;
			targetMY = (e.clientY / window.innerHeight - 0.5) * 2;
		}, { passive: true });
	}

	let mainSphere: any;
	let smallSphere: any;
	let tinySphere: any;
	let skyboxRef: any;

	const orbits = [
		{ rx: 2.8, rz: 1.8, vy: 0.6, speed: 1.0, phase: 0, yBase: 0 },
		{ rx: 3.2, rz: 1.4, vy: 0.8, speed: 1.3, phase: Math.PI * 0.7, yBase: -0.3 },
		{ rx: 2.4, rz: 1.0, vy: 0.5, speed: 1.7, phase: Math.PI * 1.4, yBase: 0.2 },
	];

	useTask(() => {
		const t = performance.now() / 1000;

		if (!envDone && t > 1 && skyboxRef) {
			skyboxRef.visible = true;
			const envTarget = pmrem.fromScene(scene, 0.04);
			scene.environment = envTarget.texture;
			skyboxRef.visible = false;
			envDone = true;
		}

		smoothScroll += (scrollProgress - smoothScroll) * 0.06;
		mouseX += (targetMX - mouseX) * 0.04;
		mouseY += (targetMY - mouseY) * 0.04;

		const scrollAngle = smoothScroll * Math.PI * 6;
		const breathe = Math.sin(t * 0.8) * 0.04;
		uBreathe.value = 1.0 + breathe * 0.5;

		if (mainSphere) {
			const o = orbits[0];
			const angle = scrollAngle * o.speed + o.phase + t * 0.08;
			mainSphere.position.x = o.rx * Math.cos(angle) + mouseX * 0.3;
			mainSphere.position.z = o.rz * Math.sin(angle);
			mainSphere.position.y = o.yBase + Math.sin(scrollAngle * 0.5 + t * 0.3) * o.vy + mouseY * -0.15;
			mainSphere.rotation.y = t * 0.15 + scrollAngle * 0.1;
			mainSphere.rotation.x = t * 0.08;
			mainSphere.scale.setScalar(1 + breathe);
		}

		if (smallSphere) {
			const o = orbits[1];
			const angle = scrollAngle * o.speed + o.phase + t * 0.06;
			smallSphere.position.x = o.rx * Math.cos(angle) + mouseX * 0.2;
			smallSphere.position.z = o.rz * Math.sin(angle);
			smallSphere.position.y = o.yBase + Math.sin(scrollAngle * 0.7 + t * 0.25 + 1) * o.vy + mouseY * -0.1;
			smallSphere.rotation.y = t * 0.2;
			smallSphere.scale.setScalar(1 + Math.sin(t * 1.1 + 2) * 0.03);
		}

		if (tinySphere) {
			const o = orbits[2];
			const angle = scrollAngle * o.speed + o.phase + t * 0.1;
			tinySphere.position.x = o.rx * Math.cos(angle) + mouseX * 0.15;
			tinySphere.position.z = o.rz * Math.sin(angle);
			tinySphere.position.y = o.yBase + Math.sin(scrollAngle * 0.9 + t * 0.2 + 2) * o.vy + mouseY * -0.08;
			tinySphere.rotation.y = t * -0.25;
		}

		camera.current.position.x = mouseX * 0.1;
		camera.current.position.y = -mouseY * 0.06;
		camera.current.lookAt(0, 0, 0);

		invalidate();
	});
</script>

<T.PerspectiveCamera makeDefault position={[0, 0, CAM_Z]} fov={FOV} />

<T.Mesh bind:ref={skyboxRef} geometry={skyGeo} material={skyMat} visible={false} />

<!-- Background plane — page capture texture goes here -->
<T.Mesh geometry={bgGeo} material={bgMat} position={[0, 0, PLANE_Z]} />

<T.AmbientLight color={0x334477} intensity={0.4} />
<T.DirectionalLight color={0xffffff} intensity={2.5} position={[3, 2, 4]} />
<T.PointLight color={0x8899cc} intensity={1.2} position={[-3, 1.5, -2]} />
<T.PointLight color={0x6677aa} intensity={0.5} position={[0, -3, 1]} />

<T.Mesh bind:ref={mainSphere} geometry={mainGeo} material={glassMat} />
<T.Mesh bind:ref={smallSphere} geometry={smallGeo} material={glassMat} position={[-2, -0.5, 0.5]} />
<T.Mesh bind:ref={tinySphere} geometry={tinyGeo} material={glassMat} position={[1.5, 0.8, -0.3]} />
