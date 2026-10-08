// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.webcam

import android.graphics.SurfaceTexture
import android.opengl.EGL14
import android.opengl.EGLConfig
import android.opengl.EGLContext
import android.opengl.EGLDisplay
import android.opengl.EGLExt
import android.opengl.EGLSurface
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.FloatBuffer
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Renders CameraX frames from an input [SurfaceTexture] into a [MediaCodec]
 * encoder input [Surface], applying sensor-to-display rotation and a 16:9
 * center-crop (`FILL_CENTER`) so the encoded H.264 stream is always upright
 * and fills the frame in both portrait and landscape phone orientations.
 */
internal class GlSurfacePipe(
    private val srcWidth: Int,
    private val srcHeight: Int,
    private val dstWidth: Int,
    private val dstHeight: Int,
    private val encoderInputSurface: Surface,
    initialRotationDegrees: Int,
    initialMirror: Boolean,
) : SurfaceTexture.OnFrameAvailableListener {

    private val thread = HandlerThread("webcam-gl-${dstWidth}x${dstHeight}").apply { start() }
    private val handler = Handler(thread.looper)

    @Volatile private var rotationDegrees: Int = ((initialRotationDegrees % 360) + 360) % 360
    @Volatile private var mirrorHorizontally: Boolean = initialMirror
    @Volatile private var released = false

    private var eglDisplay: EGLDisplay = EGL14.EGL_NO_DISPLAY
    private var eglContext: EGLContext = EGL14.EGL_NO_CONTEXT
    private var eglSurface: EGLSurface = EGL14.EGL_NO_SURFACE
    private var oesTexId: Int = 0
    private var program: Int = 0
    private var aPositionLoc: Int = 0
    private var aTexCoordLoc: Int = 0
    private var uStMatrixLoc: Int = 0

    private var surfaceTexture: SurfaceTexture? = null
    var inputSurface: Surface? = null
        private set

    private val stMatrix = FloatArray(16)
    private val quadBuffer: FloatBuffer = ByteBuffer
        .allocateDirect(4 * 4 * 4)
        .order(ByteOrder.nativeOrder())
        .asFloatBuffer()

    init {
        val ready = CountDownLatch(1)
        handler.post {
            try {
                initGl()
            } finally {
                ready.countDown()
            }
        }
        ready.await(3, TimeUnit.SECONDS)
    }

    fun updateTransform(rotationDeg: Int, mirror: Boolean) {
        rotationDegrees = ((rotationDeg % 360) + 360) % 360
        mirrorHorizontally = mirror
    }

    override fun onFrameAvailable(st: SurfaceTexture?) {
        if (released) return
        handler.post { drawFrame() }
    }

    fun release() {
        if (released) return
        released = true
        val done = CountDownLatch(1)
        handler.post {
            try {
                releaseGl()
            } finally {
                done.countDown()
            }
        }
        done.await(1, TimeUnit.SECONDS)
        thread.quitSafely()
    }

    private fun initGl() {
        eglDisplay = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
        val version = IntArray(2)
        EGL14.eglInitialize(eglDisplay, version, 0, version, 1)

        val attribList = intArrayOf(
            EGL14.EGL_RED_SIZE, 8,
            EGL14.EGL_GREEN_SIZE, 8,
            EGL14.EGL_BLUE_SIZE, 8,
            EGL14.EGL_ALPHA_SIZE, 8,
            EGL14.EGL_RENDERABLE_TYPE, EGL14.EGL_OPENGL_ES2_BIT,
            EGLExt.EGL_RECORDABLE_ANDROID, 1,
            EGL14.EGL_NONE,
        )
        val configs = arrayOfNulls<EGLConfig>(1)
        val numConfigs = IntArray(1)
        EGL14.eglChooseConfig(eglDisplay, attribList, 0, configs, 0, 1, numConfigs, 0)
        val config = configs[0] ?: return

        val ctxAttribs = intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION, 2, EGL14.EGL_NONE)
        eglContext = EGL14.eglCreateContext(eglDisplay, config, EGL14.EGL_NO_CONTEXT, ctxAttribs, 0)

        val surfaceAttribs = intArrayOf(EGL14.EGL_NONE)
        eglSurface = EGL14.eglCreateWindowSurface(eglDisplay, config, encoderInputSurface, surfaceAttribs, 0)
        EGL14.eglMakeCurrent(eglDisplay, eglSurface, eglSurface, eglContext)

        program = createProgram(VERTEX_SHADER, FRAGMENT_SHADER)
        aPositionLoc = GLES20.glGetAttribLocation(program, "aPosition")
        aTexCoordLoc = GLES20.glGetAttribLocation(program, "aTexCoord")
        uStMatrixLoc = GLES20.glGetUniformLocation(program, "uStMatrix")

        val texIds = IntArray(1)
        GLES20.glGenTextures(1, texIds, 0)
        oesTexId = texIds[0]
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, oesTexId)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MIN_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_MAG_FILTER, GLES20.GL_LINEAR)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_S, GLES20.GL_CLAMP_TO_EDGE)
        GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, GLES20.GL_TEXTURE_WRAP_T, GLES20.GL_CLAMP_TO_EDGE)

        val st = SurfaceTexture(oesTexId)
        st.setDefaultBufferSize(srcWidth, srcHeight)
        st.setOnFrameAvailableListener(this, handler)
        surfaceTexture = st
        inputSurface = Surface(st)
    }

    private fun drawFrame() {
        val st = surfaceTexture ?: return
        if (eglDisplay === EGL14.EGL_NO_DISPLAY || eglSurface === EGL14.EGL_NO_SURFACE) return

        runCatching {
            st.updateTexImage()
            st.getTransformMatrix(stMatrix)
        }.onFailure { return }

        val rot = rotationDegrees
        val mirror = mirrorHorizontally

        // After rotating the source buffer by `rot`, its upright dimensions are:
        val uprightW = if (rot % 180 != 0) srcHeight.toFloat() else srcWidth.toFloat()
        val uprightH = if (rot % 180 != 0) srcWidth.toFloat() else srcHeight.toFloat()
        val srcAspect = (uprightW / uprightH.coerceAtLeast(1f)).coerceAtLeast(0.01f)
        val dstAspect = (dstWidth.toFloat() / dstHeight.coerceAtLeast(1).toFloat()).coerceAtLeast(0.01f)

        // FILL_CENTER crop factors in upright output space:
        val scaleX: Float
        val scaleY: Float
        if (srcAspect > dstAspect) {
            scaleX = dstAspect / srcAspect
            scaleY = 1f
        } else {
            scaleX = 1f
            scaleY = srcAspect / dstAspect
        }

        updateQuadVertices(rot, scaleX, scaleY, mirror)

        GLES20.glViewport(0, 0, dstWidth, dstHeight)
        GLES20.glClearColor(0f, 0f, 0f, 1f)
        GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)

        GLES20.glUseProgram(program)
        GLES20.glActiveTexture(GLES20.GL_TEXTURE0)
        GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES, oesTexId)
        GLES20.glUniformMatrix4fv(uStMatrixLoc, 1, false, stMatrix, 0)

        quadBuffer.position(0)
        GLES20.glVertexAttribPointer(aPositionLoc, 2, GLES20.GL_FLOAT, false, 16, quadBuffer)
        GLES20.glEnableVertexAttribArray(aPositionLoc)

        quadBuffer.position(2)
        GLES20.glVertexAttribPointer(aTexCoordLoc, 2, GLES20.GL_FLOAT, false, 16, quadBuffer)
        GLES20.glEnableVertexAttribArray(aTexCoordLoc)

        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP, 0, 4)

        val tsNs = st.timestamp
        if (tsNs > 0L) {
            EGLExt.eglPresentationTimeANDROID(eglDisplay, eglSurface, tsNs)
        }
        EGL14.eglSwapBuffers(eglDisplay, eglSurface)
    }

    private fun updateQuadVertices(rot: Int, scaleX: Float, scaleY: Float, mirror: Boolean) {
        // 4 corners of clip space: (-1,-1), (1,-1), (-1,1), (1,1)
        val clipX = floatArrayOf(-1f, 1f, -1f, 1f)
        val clipY = floatArrayOf(-1f, -1f, 1f, 1f)
        quadBuffer.position(0)
        for (i in 0 until 4) {
            val vx = clipX[i]
            val vy = clipY[i]
            // Centered upright coordinates in [-0.5, +0.5], scaled for aspect-fill crop:
            val sx = (if (mirror) -vx else vx) * 0.5f * scaleX
            val sy = vy * 0.5f * scaleY
            // Map upright output point (sx, sy) back to unrotated buffer coordinates (bx, by)
            // for a clockwise image rotation of `rot` degrees:
            val bx: Float
            val by: Float
            when (rot) {
                90 -> {
                    bx = -sy
                    by = sx
                }
                180 -> {
                    bx = -sx
                    by = -sy
                }
                270 -> {
                    bx = sy
                    by = -sx
                }
                else -> {
                    bx = sx
                    by = sy
                }
            }
            quadBuffer.put(vx)
            quadBuffer.put(vy)
            quadBuffer.put(bx + 0.5f)
            quadBuffer.put(by + 0.5f)
        }
        quadBuffer.position(0)
    }

    private fun releaseGl() {
        runCatching { inputSurface?.release() }
        inputSurface = null
        runCatching {
            surfaceTexture?.setOnFrameAvailableListener(null)
            surfaceTexture?.release()
        }
        surfaceTexture = null

        if (eglDisplay !== EGL14.EGL_NO_DISPLAY) {
            EGL14.eglMakeCurrent(eglDisplay, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_SURFACE, EGL14.EGL_NO_CONTEXT)
            if (oesTexId != 0) {
                GLES20.glDeleteTextures(1, intArrayOf(oesTexId), 0)
                oesTexId = 0
            }
            if (program != 0) {
                GLES20.glDeleteProgram(program)
                program = 0
            }
            if (eglSurface !== EGL14.EGL_NO_SURFACE) {
                EGL14.eglDestroySurface(eglDisplay, eglSurface)
                eglSurface = EGL14.EGL_NO_SURFACE
            }
            if (eglContext !== EGL14.EGL_NO_CONTEXT) {
                EGL14.eglDestroyContext(eglDisplay, eglContext)
                eglContext = EGL14.EGL_NO_CONTEXT
            }
            EGL14.eglTerminate(eglDisplay)
            eglDisplay = EGL14.EGL_NO_DISPLAY
        }
    }

    companion object {
        private const val VERTEX_SHADER = """
            uniform mat4 uStMatrix;
            attribute vec4 aPosition;
            attribute vec4 aTexCoord;
            varying vec2 vTexCoord;
            void main() {
                gl_Position = aPosition;
                vTexCoord = (uStMatrix * aTexCoord).xy;
            }
        """

        private const val FRAGMENT_SHADER = """
            #extension GL_OES_EGL_image_external : require
            precision mediump float;
            varying vec2 vTexCoord;
            uniform samplerExternalOES sTexture;
            void main() {
                gl_FragColor = texture2D(sTexture, vTexCoord);
            }
        """

        private fun compileShader(type: Int, src: String): Int {
            val shader = GLES20.glCreateShader(type)
            GLES20.glShaderSource(shader, src)
            GLES20.glCompileShader(shader)
            return shader
        }

        private fun createProgram(vsSource: String, fsSource: String): Int {
            val vs = compileShader(GLES20.GL_VERTEX_SHADER, vsSource)
            val fs = compileShader(GLES20.GL_FRAGMENT_SHADER, fsSource)
            val prog = GLES20.glCreateProgram()
            GLES20.glAttachShader(prog, vs)
            GLES20.glAttachShader(prog, fs)
            GLES20.glLinkProgram(prog)
            return prog
        }
    }
}
